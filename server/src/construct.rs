use std::{sync::{atomic::{AtomicBool, AtomicUsize, Ordering, AtomicI32}, Arc}, borrow::Cow};

use anyhow::{Context, Ok};
use serde::{Serialize, Deserialize};
use swarmbot_interfaces::types::BlockState;
use tokio::task::AbortHandle;
use tracing::{error, info, trace};
use typetag::serde;

use crate::{blocks::{Vec3, Position, World, Block, SharedWorld, Direction}, mine::{ChunkedTask, fill}, turtle::{TurtleCommander, TurtleCommandResponse, TurtleCommand}, tasks::{Task, TaskState}, vendored::schematic::Schematic};

fn schematic2world(region: &Schematic) -> anyhow::Result<World> {
    let mut world = World::new();

    let min = region.origin().context("bad schematic")?;

    for (position, block) in region.blocks() {

        let name = match block {
                BlockState::AIR => None,
                BlockState(20) => None, // Glass
                BlockState(102) => None, // Glass pane
                BlockState(95) => None, // Stained glass
                BlockState(160) => None, // Stained glass pane
                // who cares
                _ => Some("terrestria:hemlock_planks")
            }.map(|s| s.to_string());

        if let Some(name) = name {
            let block = Block {
                name,
                pos: position - min,
            };
            world.set(block);
        }
    }

    Ok(world)
}

#[derive(Serialize, Deserialize,Clone)]
pub struct BuildSimple {
    pos: Vec3,
    size: Vec3,
    #[serde(skip)]
    region: Option<SharedWorld>,
    /// Input chest with the block to use, assumed infinite
    input: Position,
    #[serde(skip_deserializing)]
    miners: Arc<AtomicUsize>,
    progress: Arc<AtomicI32>,
    height: i32,
}

impl BuildSimple {
    pub fn new(position: Vec3, schematic: &Schematic, input: Position) -> Self {
        let size = Vec3::new(
            schematic.width() as i32,
            schematic.height() as i32,
            schematic.length() as i32,
        );
        Self {
            pos: position,
            size,
            region: Some(SharedWorld::from_world(schematic2world(schematic).unwrap())),
            input,
            miners: Default::default(),
            progress: Default::default(),
            height: size.y,
        }
    }

    async fn place_block(&self, turtle: TurtleCommander, at: Vec3) -> Option<()> {
        let mut near = turtle.goto_adjacent(at).await?;
        while let TurtleCommandResponse::Failure = turtle.execute(near.place(at)?).await.ret {
            trace!("failed, looking for blocks");
            if let Some(slot) = turtle.inventory().await.iter().enumerate()
                .filter(|n| n.1.clone().is_some_and(|s| s.count > 0))
                    .map(|n| n.0).next() {
                turtle.execute(TurtleCommand::Select(slot as u32 + 1)).await;
            } else {
                trace!("docking");
                turtle.goto(self.input).await;
                for _ in 1..=16 {
                    turtle.execute(TurtleCommand::SuckFront(64)).await;
                }
                near = turtle.goto_adjacent(at).await?;
            }
        }

        Some(())
    }

    async fn build_layer(&self, turtle: TurtleCommander, layer: i32) -> Option<()> {
        let layer_size = Vec3::new(self.size.x, 1, self.size.z);

        // assume the layer is empty for better pathfinding
        let mut world = turtle.world().lock_mut().await;
        for point in (0..layer_size.product()).map(|n| fill(layer_size, n)) {
            if let None = world.get(point) {
                world.set(Block { name: "minecraft:air".into(), pos: point })
            }
        }
        drop(world);

        for point in (0..layer_size.product())
            .map(|n| fill(layer_size, n)) {
            let point = point + Vec3::y() * layer;
            trace!("block {point}");

            if self.region.as_ref()?.get(point).await.is_none() {
                trace!("empty: {:?}", self.region.as_ref()?.get(point).await);
                continue;
            }

            let point = point + self.pos;

            if turtle.world().occupied(point).await {
                trace!("already full: {:?}", turtle.world().get(point).await);
                continue;
            }

            self.place_block(turtle.clone(), point).await;
        }
        Some(())
    }
}

#[serde]
impl Task for BuildSimple {
    fn run(&mut self,turtle:TurtleCommander) -> AbortHandle {
        let owned = self.clone();

        tokio::spawn(async move {
            if turtle.fuel() < 5000 {
                turtle.dock().await;
            }
            let layer = owned.progress.fetch_add(1, Ordering::AcqRel);
            if owned.height < layer {
                error!("scheduled layer out of range");
                return;
            }
            info!("layer {}", layer);
            if let None = owned.build_layer(turtle, layer).await {
                error!("building layer {} failed", layer);
                owned.progress.fetch_sub(1, Ordering::AcqRel);
            } else {
                trace!("building layer {} successful", layer);
            }
            owned.miners.fetch_sub(1, Ordering::AcqRel);
        }).abort_handle()
    }

    fn poll(&mut self) -> TaskState {
        if self.region.is_none() {
            error!("attempted to restart schematic printing, which is unimplemented");
            return TaskState::Complete;
        }

        let layer = self.progress.load(Ordering::SeqCst);

        if layer > self.height {
            return TaskState::Complete;
        }

        let only = self.miners.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            if n < 1 {
                Some(n+1)
            }else {
                None
            }
        }).is_ok();

        if only {
            return TaskState::Ready(Position::new(self.pos, Direction::North));
        }
        TaskState::Waiting
    }
}
