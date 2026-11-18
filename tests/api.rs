//! The API's answers that come before it touches the database: who may ask,
//! what a request must look like, and the parts that need no data. The
//! database handle points nowhere, so a test that reached it would fail.

use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use mongodb::{
    Client,
    options::{ClientOptions, ServerAddress},
};
use pasetors::{
    claims::Claims,
    keys::{Generate, SymmetricKey},
    local,
    paserk::FormatAsPaserk,
    version4::V4,
};
use serde_json::{Value, json};
use tower::ServiceExt;

const CLIENT_ID: &str = "test-bikeqr-client-id";

struct Harness {
    app: Router,
    key: SymmetricKey<V4>,
}

fn harness() -> Harness {
    let key = SymmetricKey::<V4>::generate().unwrap();
    let mut paserk = String::new();
    key.fmt(&mut paserk).unwrap();
    let options = ClientOptions::builder()
        .hosts(vec![ServerAddress::Tcp { host: "127.0.0.1".into(), port: Some(9) }])
        .server_selection_timeout(Duration::from_millis(50))
        .build();
    let db = Client::with_options(options).unwrap().database("nowhere");
    let verifier = sg_bike_qr::Verifier::new(&paserk, CLIENT_ID).unwrap();
    Harness { app: sg_bike_qr::app(db, verifier), key }
}

impl Harness {
    fn token(&self, email: &str) -> String {
        let mut claims = Claims::new().unwrap();
        claims.audience(CLIENT_ID).unwrap();
        claims.add_additional("email", email).unwrap();
        local::encrypt(&self.key, &claims, None, None).unwrap()
    }

    async fn call(&self, request: Request<Body>) -> (StatusCode, Value, Option<String>) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let location = response.headers().get(header::LOCATION).map(|value| value.to_str().unwrap().to_owned());
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), location)
    }

    async fn get(&self, path: &str, token: Option<&str>) -> (StatusCode, Value, Option<String>) {
        let mut request = Request::get(path);
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        self.call(request.body(Body::empty()).unwrap()).await
    }

    async fn report(&self, body: Value, token: Option<&str>) -> (StatusCode, Value) {
        let mut request = Request::post("/api/scans").header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let (status, body, _) = self.call(request.body(Body::from(body.to_string())).unwrap()).await;
        (status, body)
    }
}

fn a_report(code: &str) -> Value {
    json!({ "code": code, "lat": 1.2841, "lng": 103.8515, "accuracy": 8.0, "source": "manual" })
}

#[tokio::test]
async fn formats_need_no_sign_in_and_name_both_stickers() {
    let h = harness();
    let (status, body, _) = h.get("/api/formats", None).await;
    assert_eq!(status, StatusCode::OK);
    let operators: Vec<&str> = body.as_array().unwrap().iter().map(|f| f["operator"].as_str().unwrap()).collect();
    assert_eq!(operators, ["lta", "helloride"]);
    assert_eq!(body[1]["example"], "GRAB-001");
    assert_eq!(body[1]["head"], 4);
    assert_eq!(body[1]["tailDigitsOnly"], true);
}

#[tokio::test]
async fn reading_and_reporting_need_a_sign_in() {
    let h = harness();
    let (status, body, _) = h.get("/api/near?lat=1.28&lng=103.85&radius_m=500", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "Sign in with Aero first.");
    let (status, _) = h.report(a_report("ABC-1234"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_bad_or_foreign_token_is_a_401_so_the_page_forgets_it() {
    let h = harness();
    let (status, body, _) = h.get("/api/near?lat=1.28&lng=103.85&radius_m=500", Some("v4.local.nope")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "Your sign-in has expired. Sign in again.");
    let other = harness();
    let (status, _) = h.report(a_report("ABC-1234"), Some(&other.token("ada@example.com"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reports_must_be_a_real_code_from_singapore_with_a_good_fix() {
    let h = harness();
    let token = h.token("ada@example.com");
    for (body, needle) in [
        (a_report("GRAB001"), "isn't a parking code"),
        (a_report("ABCD-1234"), "isn't a parking code"),
        (a_report("GRAB-00A"), "isn't a parking code"),
        (json!({ "code": "ABC-1234", "lat": 1.6, "lng": 103.8, "accuracy": 8.0, "source": "manual" }), "inside Singapore"),
        (json!({ "code": "ABC-1234", "lat": 1.28, "lng": 103.85, "accuracy": 250.0, "source": "manual" }), "GPS fix"),
        (json!({ "code": "ABC-1234", "lat": 1.28, "lng": 103.85, "accuracy": 8.0, "source": "guess" }), "source"),
    ] {
        let (status, answer) = h.report(body.clone(), Some(&token)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let message = answer["error"].as_str().unwrap();
        assert!(message.contains(needle), "{body} → {message}");
    }
}

#[tokio::test]
async fn the_code_error_shows_both_shapes() {
    let h = harness();
    let (_, answer) = h.report(a_report("nope"), Some(&h.token("ada@example.com"))).await;
    assert_eq!(answer["error"], "That isn't a parking code. Codes look like ABC-1234 (LTA) or GRAB-001 (HelloRide).");
}

#[tokio::test]
async fn riding_takes_any_or_a_known_operator() {
    let h = harness();
    let token = h.token("ada@example.com");
    let (status, body, _) = h.get("/api/near?lat=1.28&lng=103.85&radius_m=500&riding=sgbike", Some(&token)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "riding must be any or one of helloride, anywheel.");
    let (status, _, _) = h.get("/api/near?lat=91&lng=103.85&radius_m=500", Some(&token)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_sign_in_starts_at_aero_with_this_apps_client_id() {
    let h = harness();
    let (status, _, location) = h.get("/api/sign-in?state=abc-123", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        location.as_deref(),
        Some("https://aero.oops.wtf/login-with-aero?client_id=test-bikeqr-client-id&state=abc-123&ttl=2592000")
    );
    for bad in ["/api/sign-in", "/api/sign-in?state=", "/api/sign-in?state=a%20b", "/api/sign-in?state=a%26client_id%3Devil"] {
        let (status, _, _) = h.get(bad, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}
