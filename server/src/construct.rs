use std::{sync::{atomic::{AtomicBool, AtomicUsize, Ordering, AtomicI32}, Arc}, borrow::Cow};

use rustmatica::{Region, Litematic, BlockState, util::UVec3};
use serde::{Serialize, Deserialize};
use tokio::task::AbortHandle;
use tracing::{error, info, trace};
use typetag::serde;

use crate::{blocks::{Vec3, Position, World, Block, SharedWorld, Direction}, mine::{ChunkedTask, fill}, turtle::{TurtleCommander, TurtleCommandResponse, TurtleCommand}, tasks::{Task, TaskState}};

fn region2world<'a>(region: &'a Region) -> World {
    let mut world = World::new();
    let min = Vec3::new(
        region.min_x() as i32,
        region.min_y() as i32,
        region.min_z() as i32,
    );

    let max = Vec3::new(
        region.max_x() as i32 + 1,
        region.max_y() as i32 + 1,
        region.max_z() as i32 + 1,
    );

    let area = max - min;
    info!("area {}", area);

    // region.blocks() is broken (or how I was using it), which cost me quite some time TODO: make a pr

    for position in (0..area.product()).map(|n| fill(area, n)) {
        let block = UVec3::new(position.x as usize, position.y as usize, position.z as usize);
        let block = region.get_block(block);

        println!("{:#?}, {}", block, position);

        let name = match block {
                BlockState::Air => None,
                BlockState::Stone => Some("minecraft:stone"),
                // who cares
                _ => Some("terrestria:hemlock_planks")
            }.map(|s| s.to_string());

        if let Some(name) = name {
            println!("{:#?}, {:?}", name, block);
            let block = Block {
                name,
                pos: position - min,
            };
            world.set(block);
        }
    }

    world
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
    pub fn new<'a>(position: Vec3, schematic: &'a Region, input: Position) -> Self {
        let size = Vec3::new(
            (1 + schematic.max_x() - schematic.min_x()) as i32,
            (1 + schematic.max_y() - schematic.min_y()) as i32,
            (1 + schematic.max_z() - schematic.min_z()) as i32,
        );
        Self {
            pos: position,
            size,
            region: Some(SharedWorld::from_world(region2world(schematic))),
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

        if layer == self.height {
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
