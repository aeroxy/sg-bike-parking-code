//! The page and the API on one local port, for development, against the
//! database `MONGO_DB` names: a real one, so mind what you report.
//!
//! Files are read from `public/` on every request, so an edit shows on
//! reload; the chip-in QR is there once `bun scripts/paynow-qr.mjs` has made
//! it. Signing in ends at the deployed callback, so a local page can't sign
//! in by itself; open `public/index.html` from disk instead to try the page
//! against its built-in sample data.

use axum::{
    http::header::CONTENT_TYPE,
    response::{Html, IntoResponse},
    routing::get,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    let uri = std::env::var("MONGO_DB").map_err(|_| "set MONGO_DB (see .env.example)")?;
    let listen = std::env::var("LISTEN").unwrap_or_else(|_| "127.0.0.1:8787".to_owned());
    let db = sg_bike_qr::connect(&uri).await?;
    let page = get(|| async {
        let html = tokio::fs::read_to_string("public/index.html").await;
        Html(html.unwrap_or_else(|err| format!("public/index.html: {err}")))
    });
    let qr = get(|| async {
        match tokio::fs::read("public/paynow.png").await {
            Ok(png) => ([(CONTENT_TYPE, "image/png")], png).into_response(),
            Err(_) => axum::http::StatusCode::NOT_FOUND.into_response(),
        }
    });
    let app = sg_bike_qr::app(db, sg_bike_qr::Verifier::aero())
        .route("/", page.clone())
        .route("/auth/aero", page)
        .route("/paynow.png", qr);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    println!("SG Bike QR on http://{listen}");
    axum::serve(listener, app).await?;
    Ok(())
}
