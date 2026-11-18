//! The page and the API on one local port, for development, against the
//! database `MONGO_DB` names: a real one, so mind what you report.
//!
//! The page is read from disk on every request, so an edit shows on reload.
//! Signing in ends at the deployed callback, so a local page can't sign in
//! by itself; open `public/index.html` from disk instead to try the page
//! against its built-in sample data.

use axum::{response::Html, routing::get};

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
    let app = sg_bike_qr::app(db, sg_bike_qr::Verifier::aero()).route("/", page.clone()).route("/auth/aero", page);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    println!("SG Bike QR on http://{listen}");
    axum::serve(listener, app).await?;
    Ok(())
}
