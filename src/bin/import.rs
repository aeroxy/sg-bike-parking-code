//! Moves the app's data from Aero's Firestore into MongoDB, once, at the
//! switch from Aero to Vercel.
//!
//! Reads what `scripts/export.py` printed (one Firestore document per line),
//! reports anything odd in it, writes every document into the database
//! `MONGO_DB` names, then reads them all back and compares.
//!
//!     cargo run --bin import -- export.ndjson [--dry-run]
//!
//! Firestore wins: a document already in MongoDB under the same id is
//! replaced with Firestore's. Nothing is ever deleted. That makes a second
//! run safe before the switch, and wrong after it, once the new app has moved
//! pins: so the import refuses to run once MongoDB holds any report the new
//! app made (those have ObjectId ids; Firestore's are strings).

use std::{collections::HashMap, error::Error, process::ExitCode};

use futures_util::{StreamExt, TryStreamExt, stream};
use mongodb::{
    Database,
    bson::{Bson, DateTime, Document, doc},
};
use serde_json::Value;
use sg_bike_qr::{LOCATIONS, LTA, SCANS, in_singapore, median, number, parse_code, point};

/// Writes in flight at once.
const PARALLEL: usize = 16;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("import failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// One Firestore document as the export printed it.
struct Exported {
    collection: String,
    id: String,
    create_time: DateTime,
    update_time: DateTime,
    fields: Document,
}

async fn run() -> Result<bool, Box<dyn Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let path = args.iter().find(|arg| !arg.starts_with("--")).ok_or("usage: import <export.ndjson> [--dry-run]")?;

    let mut scans = Vec::new();
    let mut locations = Vec::new();
    for (n, line) in std::fs::read_to_string(path)?.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let exported = exported(line).map_err(|err| format!("line {}: {err}", n + 1))?;
        match exported.collection.as_str() {
            "qr_scans" => scans.push(scan_document(exported)),
            "qr_locations" => locations.push(location_document(exported)),
            other => return Err(format!("line {}: unknown collection {other}", n + 1).into()),
        }
    }
    println!("read {} scans and {} locations", scans.len(), locations.len());
    report(&scans, &locations);

    if dry_run {
        println!("dry run: nothing written");
        return Ok(true);
    }
    let uri = std::env::var("MONGO_DB").map_err(|_| "set MONGO_DB (see .env.example)")?;
    let db = sg_bike_qr::connect(&uri).await?;
    let made_here = db.collection::<Document>(SCANS).count_documents(doc! { "_id": { "$type": "objectId" } }).await?;
    if made_here > 0 {
        return Err(format!(
            "{made_here} reports in MongoDB came from the new app; importing now would move their pins back. Nothing written."
        )
        .into());
    }
    write(&db, SCANS, &scans).await?;
    write(&db, LOCATIONS, &locations).await?;
    let same = verify(&db, SCANS, &scans).await? & verify(&db, LOCATIONS, &locations).await? & spot_check(&db, &locations).await?;
    println!("{}", if same { "verified: MongoDB matches the export" } else { "MISMATCHES above" });
    Ok(same)
}

fn exported(line: &str) -> Result<Exported, String> {
    let value: Value = serde_json::from_str(line).map_err(|err| err.to_string())?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).ok_or(format!("no {key}"));
    let time = |key: &str| DateTime::parse_rfc3339_str(text(key)?).map_err(|err| format!("{key}: {err}"));
    let fields = value.get("fields").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut document = Document::new();
    for (key, typed) in &fields {
        document.insert(key.clone(), firestore_value(typed).map_err(|err| format!("{key}: {err}"))?);
    }
    Ok(Exported {
        collection: text("collection")?.to_owned(),
        id: text("id")?.to_owned(),
        create_time: time("createTime")?,
        update_time: time("updateTime")?,
        fields: document,
    })
}

/// A Firestore REST value (`{"integerValue": "3"}` and the like) as BSON.
fn firestore_value(typed: &Value) -> Result<Bson, String> {
    let (kind, inner) = typed.as_object().and_then(|object| object.iter().next()).ok_or("empty value")?;
    let bad = || format!("bad {kind}");
    Ok(match kind.as_str() {
        "stringValue" => Bson::String(inner.as_str().ok_or_else(bad)?.to_owned()),
        "integerValue" => Bson::Int64(inner.as_str().and_then(|text| text.parse().ok()).ok_or_else(bad)?),
        "doubleValue" => Bson::Double(inner.as_f64().ok_or_else(bad)?),
        "booleanValue" => Bson::Boolean(inner.as_bool().ok_or_else(bad)?),
        "nullValue" => Bson::Null,
        "timestampValue" => {
            Bson::DateTime(DateTime::parse_rfc3339_str(inner.as_str().ok_or_else(bad)?).map_err(|_| bad())?)
        }
        "mapValue" => {
            let mut document = Document::new();
            for (key, value) in inner.get("fields").and_then(Value::as_object).into_iter().flatten() {
                document.insert(key.clone(), firestore_value(value)?);
            }
            Bson::Document(document)
        }
        "arrayValue" => Bson::Array(
            inner.get("values").and_then(Value::as_array).into_iter().flatten().map(firestore_value).collect::<Result<_, _>>()?,
        ),
        other => return Err(format!("unsupported {other}")),
    })
}

/// Coordinates are doubles, even where a client sent a whole number.
fn as_double(document: &mut Document, key: &str) {
    if let Some(value) = number(document, key) {
        document.insert(key, value);
    }
}

fn scan_document(exported: Exported) -> Document {
    let mut document = doc! { "_id": exported.id };
    document.extend(exported.fields);
    for key in ["lat", "lng", "accuracy"] {
        as_double(&mut document, key);
    }
    document.insert("createdAt", exported.create_time);
    document
}

fn location_document(exported: Exported) -> Document {
    let mut document = doc! { "_id": &exported.id };
    document.extend(exported.fields);
    // An index artifact of Aero's geo queries; MongoDB's own index replaces it.
    document.remove("_geohash");
    for key in ["lat", "lng"] {
        as_double(&mut document, key);
    }
    let code = document.get_str("code").unwrap_or(&exported.id).to_owned();
    let operator = parse_code(&code).map_or(LTA, |(_, format)| format.operator);
    document.insert("code", &code);
    document.insert("operator", operator);
    if let (Some(lat), Some(lng)) = (number(&document, "lat"), number(&document, "lng")) {
        document.insert("location", point(lat, lng));
    }
    document.insert("createdAt", exported.create_time);
    document.insert("updatedAt", exported.update_time);
    document
}

/// Prints what looks wrong in the export, changing nothing.
fn report(scans: &[Document], locations: &[Document]) {
    let code = |document: &Document| document.get_str("code").unwrap_or("").to_owned();
    for (name, documents) in [("scan", scans), ("location", locations)] {
        for document in documents {
            let id = document.get("_id").map(ToString::to_string).unwrap_or_default();
            let (lat, lng) = (number(document, "lat"), number(document, "lng"));
            match (lat, lng) {
                (Some(lat), Some(lng)) if in_singapore(lat, lng) => {}
                (Some(_), Some(_)) => println!("  {name} {id}: outside Singapore"),
                _ => println!("  {name} {id}: no coordinates"),
            }
            let code = code(document);
            if parse_code(&code).is_none() {
                println!("  {name} {id}: {code:?} is no sticker's code");
            }
            if name == "location" && id.trim_matches('"') != code {
                println!("  location {id}: id and code differ ({code:?})");
            }
        }
    }

    // Before HelloRide's stickers were supported, typing GRAB001 put the dash
    // after three letters: GRA-B001 passed as an LTA code.
    let suspects: Vec<String> = locations
        .iter()
        .map(code)
        .filter(|code| {
            let bytes = code.as_bytes();
            bytes.len() == 8
                && bytes[..3].iter().all(u8::is_ascii_uppercase)
                && bytes[3] == b'-'
                && bytes[4].is_ascii_uppercase()
                && bytes[5..].iter().all(u8::is_ascii_digit)
        })
        .collect();
    println!("possibly HelloRide codes saved as LTA ones (letter after the dash, then 3 digits): {}", suspects.len());
    for suspect in &suspects {
        println!("  {suspect}");
    }

    // Each pin against the median of its own reports.
    let mut by_code: HashMap<String, (Vec<f64>, Vec<f64>)> = HashMap::new();
    for scan in scans {
        if let (Some(lat), Some(lng)) = (number(scan, "lat"), number(scan, "lng")) {
            let entry = by_code.entry(code(scan)).or_default();
            entry.0.push(lat);
            entry.1.push(lng);
        }
    }
    let total: i64 = locations.iter().filter_map(|location| number(location, "numReports")).map(|n| n as i64).sum();
    println!("numReports add up to {total}; there are {} scans", scans.len());
    let (mut moved, mut miscounted, mut orphans) = (Vec::new(), Vec::new(), Vec::new());
    for location in locations {
        let code = code(location);
        let Some((lats, lngs)) = by_code.get_mut(&code) else {
            orphans.push(code);
            continue;
        };
        let reports = lats.len();
        let (lat, lng) = (median(lats).unwrap_or(f64::NAN), median(lngs).unwrap_or(f64::NAN));
        let off = |key: &str, value: f64| number(location, key).is_none_or(|stored| (stored - value).abs() > 1e-7);
        if off("lat", lat) || off("lng", lng) {
            moved.push(code.clone());
        }
        if number(location, "numReports").is_none_or(|n| n as usize != reports) {
            miscounted.push(code);
        }
    }
    let located: std::collections::HashSet<String> = locations.iter().map(code).collect();
    let unplaced: Vec<&String> = by_code.keys().filter(|code| !located.contains(*code)).collect();
    println!("pins away from their reports' median: {} {:?}", moved.len(), moved);
    println!("pins whose numReports differs from their reports: {} {:?}", miscounted.len(), miscounted);
    println!("pins with no reports: {} {:?}", orphans.len(), orphans);
    println!("reported codes with no pin: {} {:?}", unplaced.len(), unplaced);
}

async fn write(db: &Database, collection: &str, documents: &[Document]) -> Result<(), mongodb::error::Error> {
    let target = db.collection::<Document>(collection);
    stream::iter(documents)
        .map(|document| {
            let target = target.clone();
            async move {
                let id = document.get("_id").cloned().unwrap_or(Bson::Null);
                target.replace_one(doc! { "_id": id }, document).upsert(true).await
            }
        })
        .buffer_unordered(PARALLEL)
        .try_collect::<Vec<_>>()
        .await?;
    println!("wrote {} documents to {collection}", documents.len());
    Ok(())
}

/// Reads every written document back and compares it with what was written.
async fn verify(db: &Database, collection: &str, documents: &[Document]) -> Result<bool, mongodb::error::Error> {
    let ids: Vec<Bson> = documents.iter().filter_map(|document| document.get("_id").cloned()).collect();
    let stored: Vec<Document> =
        db.collection::<Document>(collection).find(doc! { "_id": { "$in": ids } }).await?.try_collect().await?;
    let stored: HashMap<String, Document> =
        stored.into_iter().map(|document| (document.get("_id").map(ToString::to_string).unwrap_or_default(), document)).collect();
    let mut same = stored.len() == documents.len();
    for document in documents {
        let id = document.get("_id").map(ToString::to_string).unwrap_or_default();
        if stored.get(&id) != Some(document) {
            println!("  {collection} {id}: differs in MongoDB");
            same = false;
        }
    }
    println!("{collection}: {} of {} read back identical", if same { documents.len() } else { 0 }, documents.len());
    Ok(same)
}

/// A nearest-first search at a few pins must find each pin first, right there.
async fn spot_check(db: &Database, locations: &[Document]) -> Result<bool, mongodb::error::Error> {
    let step = (locations.len() / 5).max(1);
    let mut same = true;
    for location in locations.iter().step_by(step).take(5) {
        let (Some(lat), Some(lng)) = (number(location, "lat"), number(location, "lng")) else { continue };
        let pipeline = [
            doc! { "$geoNear": { "near": point(lat, lng), "key": "location", "distanceField": "d", "spherical": true } },
            doc! { "$limit": 1 },
        ];
        let found: Vec<Document> = db.collection::<Document>(LOCATIONS).aggregate(pipeline).await?.try_collect().await?;
        let first = found.first();
        let distance = first.and_then(|document| number(document, "d")).unwrap_or(f64::INFINITY);
        let hit = first.and_then(|document| document.get_str("code").ok()) == location.get_str("code").ok();
        // Two stickers can share a spot; then the other may come first, at 0 m too.
        let ok = distance < 0.5 && (hit || distance < 0.01);
        println!("  near {}: {}", location.get_str("code").unwrap_or("?"), if ok { "found first" } else { "NOT found first" });
        same &= ok;
    }
    Ok(same)
}
