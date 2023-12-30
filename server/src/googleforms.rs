use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use anyhow::{Ok, Context, anyhow, Result};
use axum::{Router, routing::post, extract::State, Json};
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;
use tracing::{info, error};
use typetag::serde;

use crate::{SharedControl, mine::{Remove, ChunkedTask, Quarry}, blocks::{Vec3, Direction, Position}, tasks::{TaskState, Task}, turtle::TurtleCommander, construct::BuildSimple};

pub fn forms_api() -> Router<SharedControl> {
    Router::new()
        .route("/registerVeinMine", post(remove_vein))
        .route("/omni", post(omni))
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

#[derive(Deserialize, Debug)]
enum GoogleOmniFormMode {
    #[serde(rename(deserialize = "Schematic Building"))]
    Schematic,
    #[serde(rename(deserialize = "Vein Removal"))]
    RemoveVein,
    #[serde(rename(deserialize = "Area Removal"))]
    RemoveArea,
    #[serde(rename(deserialize = "Summon Turtle"))]
    Goto,
}

// I feel like I could use the type system more along with flatten, but no
#[derive(Deserialize, Debug)]
struct GoogleOmniForm{
    #[serde(rename(deserialize = "Select an operation"))]
    operation: GoogleOmniFormMode,
    #[serde(rename(deserialize = "X coordinate"), alias="X coordinate (from)")]
    x: String,
    #[serde(rename(deserialize = "Y coordinate"), alias="Y coordinate (from)")]
    y: String,
    #[serde(rename(deserialize = "Z coordinate"), alias="Z coordinate (from)")]
    z: String,
    #[serde(default, rename(deserialize = "Block name"))]
    block: Option<String>,
    #[serde(default, rename(deserialize = "Facing"))]
    facing: Option<Direction>,
    #[serde(default, rename(deserialize = "X coordinate (to)"))]
    x2: Option<String>,
    #[serde(rename(deserialize = "Y coordinate (to)"))]
    y2: Option<String>,
    #[serde(rename(deserialize = "Z coordinate (to)"))]
    z2: Option<String>,
    #[serde(rename(deserialize = "Upload a .litematic file"))]
    schematic: Option<Vec<String>>,
}

async fn omni(
    State(state): State<SharedControl>,
    Json(req): Json<GoogleOmniForm>,
) -> &'static str {
    info!("omni: {:?}", req);
    match omni_inner(state, req).await {
        anyhow::Result::Ok(_) => {},
        anyhow::Result::Err(e) => error!("remove vein request failed: {e}"),
    };

    "ACK"
}

async fn omni_inner(state: SharedControl, req: GoogleOmniForm) -> anyhow::Result<()> {
    let state = state.read().await;
    let mut schedule = state.tasks.lock().await;
    let position = { Vec3::new(req.x.parse()?,req.y.parse()?,req.z.parse()?) };
    match req.operation {
        GoogleOmniFormMode::Schematic => {
            let schematic = req.schematic.context("no schematic uploaded")?.get(0).context("zero schematics")?.to_owned();
            let schematic = reqwest::get(format!("https://docs.google.com/uc?export=download&id={schematic}")).await?;

            let schematic = rustmatica::Litematic::from_bytes(&schematic.bytes().await?)?;


            info!("schematic \"{}\" downloaded", &schematic.name);
            info!("{} blocks", schematic.total_blocks());
            info!("{} regions", schematic.regions.len());

            let input = Position::new(
                Vec3::new(53,73,77),
                Direction::West,
            );

            // this converts to my memory representation so it can take a while
            let builder = tokio::task::spawn_blocking(move || {
                let region = schematic.regions.get(0).context("no regions");
                Ok(BuildSimple::new(position, region?, input))
            }).await??;

            schedule.add_task(Box::new(builder));
        },
        GoogleOmniFormMode::RemoveVein => {
            let block = req.block.context("missing block name")?;
            info!("new remove {block} command from the internet at {position}");
            schedule.add_task(Box::new(Remove::new(position,block)));
        },
        GoogleOmniFormMode::RemoveArea => {
            let upper = Vec3::new(
                req.x2.context("x2")?.parse()?,
                req.y2.context("y2")?.parse()?,
                req.z2.context("z2")?.parse()?,
            );

            let quarry = Quarry::new(position, upper);
            schedule.add_task(Box::new(quarry));
        },
        GoogleOmniFormMode::Goto => {
            schedule.add_task(Box::new(Goto::new(Position::new(position, req.facing.context("missing direction")?))));
        },
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Goto {
    position: Position,
    done: Arc<AtomicBool>,
}

impl Goto {
    fn new(position: Position) -> Self {
        Self {
            position,
            done: Default::default(),
        }
    }
}

#[serde]
impl Task for Goto {
    fn run(&mut self,turtle:TurtleCommander) -> AbortHandle {
        self.done.store(true, Ordering::SeqCst);
        let position = self.position.clone();

        tokio::spawn(async move {
            turtle.goto(position).await;
        }).abort_handle()
    }

    fn poll(&mut self) -> TaskState {
        if self.done.load(Ordering::SeqCst) {
            return TaskState::Complete;
        }

        TaskState::Ready(self.position)
    }
}
