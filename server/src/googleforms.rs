use anyhow::Ok;
use axum::{Router, routing::post, extract::State, Json};
use serde::{Deserialize, Serialize};
use tracing::{info, error};

use crate::{SharedControl, mine::Remove, blocks::Vec3};

pub fn forms_api() -> Router<SharedControl> {
    Router::new()
        .route("/registerVeinMine", post(remove_vein))
}

#[derive(Serialize, Deserialize)]
struct GoogleFormsRemoveVein{
    #[serde(rename(deserialize = "Block name"))]
    block: String,
    #[serde(rename(deserialize = "X coordinate"))]
    x: String,
    #[serde(rename(deserialize = "Y coordinate"))]
    y: String,
    #[serde(rename(deserialize = "Z coordinate"))]
    z: String,
}

async fn remove_vein(
    State(state): State<SharedControl>,
    Json(req): Json<GoogleFormsRemoveVein>,
) -> &'static str {
    match remove_vein_inner(state, req).await {
        anyhow::Result::Ok(_) => {},
        anyhow::Result::Err(e) => error!("remove vein request failed: {e}"),
    };

    "ACK"
}

async fn remove_vein_inner(state: SharedControl, req: GoogleFormsRemoveVein) -> anyhow::Result<()> {
    let state = state.read().await;
    let mut schedule = state.tasks.lock().await;
    let position = { Vec3::new(req.x.parse()?,req.y.parse()?,req.z.parse()?) };
    let block = req.block;
    info!("new remove {block} command from the internet at {position}");
    schedule.add_task(Box::new(Remove::new(position,block)));
    Ok(())
}
