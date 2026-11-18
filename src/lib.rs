//! SG Bike QR's API: bike-share parking QR stickers on a map of Singapore.
//!
//! Every report of a sticker is kept (`qr_scans`), and the pin people see
//! (`qr_locations`, one per code) sits at the median of its reports, so one
//! bad GPS fix gets outvoted instead of moving the pin. The API recomputes a
//! pin whenever someone reports its code; nobody can set one directly.
//!
//! A code's shape says whose sticker it is (see `codes`): LTA's, which every
//! operator honours, or one operator's own. Pins carry that as `operator`, and
//! the map can show a rider only the stickers their operator honours.
//!
//! Reading and reporting both need a sign-in with Aero (see `auth`). Each
//! report is stamped with the reporter's email for audits; no response ever
//! carries it.

mod auth;
mod codes;
mod geo;

pub use auth::Verifier;
pub use codes::{FORMATS, LTA, parse as parse_code};
pub use geo::{in_singapore, median};

use auth::Person;

use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{FromRequestParts, Query, State},
    http::{
        StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL},
        request::Parts,
    },
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use futures_util::TryStreamExt;
use mongodb::{
    Client, Database, IndexModel,
    bson::{Bson, DateTime, Document, doc},
    options::ClientOptions,
};
use serde::{Deserialize, Serialize};

/// One row per report of a code.
pub const SCANS: &str = "qr_scans";
/// One row per code, `_id` being the code: the pin.
pub const LOCATIONS: &str = "qr_locations";

/// A report's GPS fix must be at least this good, in metres.
const MAX_ACCURACY_M: f64 = 100.0;
/// The radius a map view asks about, clamped to this range in metres.
const MIN_RADIUS_M: f64 = 250.0;
const MAX_RADIUS_M: f64 = 50_000.0;
/// The most pins one view returns, nearest first.
const NEAR_LIMIT: i64 = 50;
/// The most of a scanned QR code's text kept with a report.
const RAW_MAX: usize = 500;

/// Connects to the database `uri` names and makes sure its indexes exist.
/// Creating an index that is already there changes nothing, so every cold
/// start may do it.
pub async fn connect(uri: &str) -> Result<Database, Box<dyn std::error::Error + Send + Sync>> {
    let mut options = ClientOptions::parse(uri).await?;
    options.server_selection_timeout = Some(Duration::from_secs(10));
    if options.app_name.is_none() {
        options.app_name = Some("sg-bike-qr".to_owned());
    }
    let db = Client::with_options(options)?
        .default_database()
        .ok_or("the connection string must name the database, as in …mongodb.net/sg_bike_qr")?;
    db.collection::<Document>(LOCATIONS)
        .create_index(IndexModel::builder().keys(doc! { "location": "2dsphere" }).build())
        .await?;
    db.collection::<Document>(SCANS)
        .create_index(IndexModel::builder().keys(doc! { "code": 1 }).build())
        .await?;
    Ok(db)
}

/// `verifier` decides who a sign-in token stands for: [`Verifier::aero`] for
/// real, a test key in tests.
pub fn app(db: Database, verifier: Verifier) -> Router {
    Router::new()
        .route("/api/formats", get(formats))
        .route("/api/near", get(near))
        .route("/api/scans", post(report))
        .route("/api/sign-in", get(sign_in))
        .with_state(AppState { db, verifier: Arc::new(verifier) })
}

#[derive(Clone)]
struct AppState {
    db: Database,
    verifier: Arc<Verifier>,
}

// ---------------------------------------------------------------- identity

/// Who is asking: the person a valid sign-in token names, or nobody.
struct Reader(Option<Person>);

impl Reader {
    fn required(self) -> Result<Person, ApiError> {
        self.0.ok_or_else(|| ApiError::Unauthorized("Sign in with Aero first.".to_owned()))
    }
}

impl FromRequestParts<AppState> for Reader {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let Some(value) = parts.headers.get(AUTHORIZATION) else {
            return Ok(Reader(None));
        };
        // A token that fails is a 401, not "nobody": the page then knows to
        // forget it and sign in again.
        value
            .to_str()
            .ok()
            .and_then(|text| text.strip_prefix("Bearer "))
            .and_then(|token| state.verifier.person(token))
            .map(|person| Reader(Some(person)))
            .ok_or_else(|| ApiError::Unauthorized("Your sign-in has expired. Sign in again.".to_owned()))
    }
}

// ------------------------------------------------------------------ errors

pub enum ApiError {
    BadRequest(String),
    /// No sign-in where one is needed, or one that failed verification.
    Unauthorized(String),
    /// Not set up on this deployment, such as sign-in before registration.
    Unavailable(String),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            ApiError::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message),
            ApiError::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
            ApiError::Internal => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong. Try again.".to_owned())
            }
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl From<mongodb::error::Error> for ApiError {
    fn from(err: mongodb::error::Error) -> Self {
        // Logged here, not sent: the caller only learns that it failed.
        eprintln!("database error: {err}");
        ApiError::Internal
    }
}

// -------------------------------------------------------------------- pins

/// A pin as the page draws it. No `_owner`: who reported a code is nobody
/// else's business.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub code: String,
    #[serde(default = "lta")]
    pub operator: String,
    pub lat: f64,
    pub lng: f64,
    pub num_reports: i64,
    /// Milliseconds since the epoch.
    pub last_report_at: i64,
    /// From the point asked about, on `/api/near` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_meters: Option<f64>,
}

fn lta() -> String {
    LTA.to_owned()
}

/// What every pin projection keeps.
fn pin_fields() -> Document {
    doc! { "_id": 0, "code": 1, "operator": 1, "lat": 1, "lng": 1, "numReports": 1, "lastReportAt": 1 }
}

// ---------------------------------------------------------------- handlers

async fn formats() -> impl IntoResponse {
    ([(CACHE_CONTROL, "public, max-age=300, s-maxage=3600")], Json(FORMATS))
}

#[derive(Deserialize)]
struct NearQuery {
    lat: f64,
    lng: f64,
    radius_m: f64,
    /// `any` (the default), or an operator from `codes::RIDING`.
    riding: Option<String>,
}

#[derive(Serialize)]
struct NearList {
    codes: Vec<Pin>,
}

/// Pins within the radius, nearest first, with their distances.
async fn near(
    State(app): State<AppState>,
    reader: Reader,
    Query(query): Query<NearQuery>,
) -> Result<Json<NearList>, ApiError> {
    reader.required()?;
    let point_ok = (-90.0..=90.0).contains(&query.lat) && (-180.0..=180.0).contains(&query.lng);
    if !point_ok || query.radius_m.is_nan() {
        return Err(ApiError::BadRequest("lat, lng and radius_m must be a point and a distance.".to_owned()));
    }
    let radius = query.radius_m.clamp(MIN_RADIUS_M, MAX_RADIUS_M);
    let mut fields = pin_fields();
    fields.insert("distanceMeters", 1);
    let pipeline = [
        doc! { "$geoNear": {
            "near": { "type": "Point", "coordinates": [query.lng, query.lat] },
            "key": "location",
            "distanceField": "distanceMeters",
            "maxDistance": radius,
            "spherical": true,
            "query": riding_filter(query.riding.as_deref())?,
        }},
        doc! { "$limit": NEAR_LIMIT },
        doc! { "$project": fields },
    ];
    let codes = app.db.collection::<Document>(LOCATIONS).aggregate(pipeline).with_type::<Pin>().await?;
    Ok(Json(NearList { codes: codes.try_collect().await? }))
}

/// Which pins a rider can use: all of them, or LTA's plus their operator's.
fn riding_filter(riding: Option<&str>) -> Result<Document, ApiError> {
    match riding.unwrap_or("any") {
        "any" => Ok(doc! {}),
        operator if codes::RIDING.contains(&operator) => Ok(doc! { "operator": { "$in": [LTA, operator] } }),
        _ => Err(ApiError::BadRequest(format!("riding must be any or one of {}.", codes::RIDING.join(", ")))),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    code: String,
    lat: f64,
    lng: f64,
    /// The GPS fix's accuracy in metres.
    accuracy: f64,
    /// `scan` when the camera read the code, `manual` when typed.
    source: String,
    /// What the scanned QR code said, kept so that new kinds of sticker can
    /// be learned from real ones.
    raw: Option<String>,
    client_timestamp: Option<i64>,
}

/// Records a report and returns the code's pin as it now stands.
async fn report(
    State(app): State<AppState>,
    reader: Reader,
    Json(report): Json<Report>,
) -> Result<Json<Pin>, ApiError> {
    let person = reader.required()?;
    let Some((code, format)) = codes::parse(&report.code) else {
        let shapes: Vec<String> =
            FORMATS.iter().map(|format| format!("{} ({})", format.example, format.label)).collect();
        return Err(ApiError::BadRequest(format!("That isn't a parking code. Codes look like {}.", shapes.join(" or "))));
    };
    if !geo::in_singapore(report.lat, report.lng) {
        return Err(ApiError::BadRequest("Reports must come from inside Singapore.".to_owned()));
    }
    if !(0.0..=MAX_ACCURACY_M).contains(&report.accuracy) {
        return Err(ApiError::BadRequest(format!("The GPS fix must be within {MAX_ACCURACY_M} m.")));
    }
    if report.source != "scan" && report.source != "manual" {
        return Err(ApiError::BadRequest("source must be scan or manual.".to_owned()));
    }
    let now = DateTime::now();
    let mut scan = doc! {
        "code": &code,
        "lat": report.lat,
        "lng": report.lng,
        "accuracy": report.accuracy,
        "source": &report.source,
        "createdAt": now,
        "_owner": &person.email,
    };
    if let Some(raw) = report.raw.as_deref().map(str::trim).filter(|raw| !raw.is_empty()) {
        scan.insert("raw", raw.chars().take(RAW_MAX).collect::<String>());
    }
    if let Some(at) = report.client_timestamp {
        scan.insert("clientTimestamp", at);
    }
    app.db.collection::<Document>(SCANS).insert_one(scan).await?;
    Ok(Json(recompute(&app.db, &code, format.operator, &person.email, now).await?))
}

/// Moves a code's pin to the median of all its reports.
pub async fn recompute(
    db: &Database,
    code: &str,
    operator: &str,
    email: &str,
    now: DateTime,
) -> Result<Pin, mongodb::error::Error> {
    let mut scans = db
        .collection::<Document>(SCANS)
        .find(doc! { "code": code })
        .projection(doc! { "_id": 0, "lat": 1, "lng": 1 })
        .await?;
    let (mut lats, mut lngs) = (Vec::new(), Vec::new());
    while let Some(scan) = scans.try_next().await? {
        if let (Some(lat), Some(lng)) = (number(&scan, "lat"), number(&scan, "lng")) {
            lats.push(lat);
            lngs.push(lng);
        }
    }
    let (Some(lat), Some(lng)) = (geo::median(&mut lats), geo::median(&mut lngs)) else {
        return Err(mongodb::error::Error::custom(format!("{code} has no reports to place it")));
    };
    let pin = Pin {
        code: code.to_owned(),
        operator: operator.to_owned(),
        lat,
        lng,
        num_reports: lats.len() as i64,
        last_report_at: now.timestamp_millis(),
        distance_meters: None,
    };
    db.collection::<Document>(LOCATIONS)
        .update_one(
            doc! { "_id": code },
            doc! {
                "$set": {
                    "code": code,
                    "operator": operator,
                    "lat": lat,
                    "lng": lng,
                    "location": point(lat, lng),
                    "numReports": pin.num_reports,
                    "lastReportAt": pin.last_report_at,
                    "updatedAt": now,
                },
                "$setOnInsert": { "createdAt": now, "_owner": email },
            },
        )
        .upsert(true)
        .await?;
    Ok(pin)
}

/// A GeoJSON point, which MongoDB's `2dsphere` index wants as `[lng, lat]`.
pub fn point(lat: f64, lng: f64) -> Document {
    doc! { "type": "Point", "coordinates": [lng, lat] }
}

/// A number however BSON stored it.
pub fn number(doc: &Document, key: &str) -> Option<f64> {
    match doc.get(key)? {
        Bson::Double(value) => Some(*value),
        Bson::Int32(value) => Some(f64::from(*value)),
        Bson::Int64(value) => Some(*value as f64),
        _ => None,
    }
}

#[derive(Deserialize)]
struct SignInQuery {
    state: String,
}

/// Sends the browser to Aero's consent screen with this app's client id.
async fn sign_in(State(app): State<AppState>, Query(query): Query<SignInQuery>) -> Result<Redirect, ApiError> {
    let state = query.state;
    let url_safe = state.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !(1..=512).contains(&state.len()) || !url_safe {
        return Err(ApiError::BadRequest("state must be 1 to 512 letters, digits, - or _.".to_owned()));
    }
    let url = app
        .verifier
        .sign_in_url(&state)
        .ok_or_else(|| ApiError::Unavailable("Sign-in isn't set up yet.".to_owned()))?;
    Ok(Redirect::to(&url))
}
