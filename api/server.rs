//! The Vercel Function. `vercel.json` rewrites every `/api/*` request here,
//! and the router still sees the original path.

use tower::ServiceBuilder;
use vercel_runtime::{Error, axum::VercelLayer};

#[tokio::main]
async fn main() -> Result<(), Error> {
    let uri = std::env::var("MONGO_DB").map_err(|_| "MONGO_DB is not set")?;
    let db = sg_bike_qr::connect(&uri).await?;
    let app = ServiceBuilder::new()
        .layer(VercelLayer::new())
        .service(sg_bike_qr::app(db, sg_bike_qr::Verifier::aero()));
    vercel_runtime::run(app).await
}
