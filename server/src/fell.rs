use std::ops::Mul;

use log::{trace, warn, info};
use serde::{Serialize, Deserialize};
use time::OffsetDateTime;
use tokio::task::{JoinHandle, AbortHandle};
use typetag::serde;

use crate::{blocks::{Vec3, Position, Direction}, turtle::{TurtleCommander, TurtleCommand, TurtleCommandResponse, InventorySlot}, tasks::{Task, TaskState}, depot::Depots, mine::fill, paths::TRANSPARENT};

pub async fn fell_tree(turtle: TurtleCommander, bottom: Vec3) -> Option<()> {
    let mut log = bottom;
    loop {
        let near = turtle.goto_adjacent(log).await?;
        if turtle.world().get(log).await.is_some_and(|b| !b.name.contains("log")) {
            break;
        }
        turtle.execute(near.dig(log)?).await;
        log += Vec3::y();
    }
    Some(())
}

/// Minutes before checking
const SWEEP_DELAY: i64 = 16;

#[derive(Serialize, Deserialize, Clone)]
pub struct TreeFarm {
    position: Vec3,
    size: Vec3,
    last_sweep: OffsetDateTime,
}

impl TreeFarm {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            size: Vec3::new(5,1,2),
            last_sweep: OffsetDateTime::UNIX_EPOCH,
        }
    }

    pub async fn sweep(&self, turtle: TurtleCommander) -> Option<()> {
        let trees = self.size.product();
        let spacing = Vec3::new(2, 32, 2);
        turtle.dock().await;
        for tree in 0..trees {
            let index = fill(self.size, tree);
            let offset = index.component_mul(&spacing);
            trace!("tree {tree}; {offset:?}");
            let tree = self.position + offset;
            fell_tree(turtle.clone(), tree).await?;
        }

        // sweep across floor (not upper levels) to get saplings
        // this goes one block past the far corner and to the near corner
        let area = self.size.xz().component_mul(&spacing.xz()).product();
        for tile in 0..area {
            let offset = fill(self.size.component_mul(&Vec3::new(spacing.x, 1, spacing.z)), tile);
            let tile = self.position + offset;
            turtle.goto_adjacent(tile-Vec3::y()).await;
            turtle.execute(TurtleCommand::SuckFront(64)).await;
        }

        // scan inventory for saplings
        let mut saplings = Vec::new();
        let mut needed = trees;
        for slot in 1..=16 {
            if let TurtleCommandResponse::Item(i) = turtle.execute(TurtleCommand::ItemInfo(slot)).await.ret {
                if i.name.contains("sapling") {
                    needed -= i.count as i32;
                    saplings.push((slot,i));
                }
                if needed <= 0 { break; }
            }
        }

        if needed > 0 {
            warn!("incomplete wood harvest, {needed} saplings short");
        }

        fn pop_item(vec: &mut Vec<(u32, InventorySlot)>) -> Option<u32> {
            let mut slot = vec.pop()?;
            let index = slot.0;
            slot.1.count -= 1;
            if slot.1.count > 0 {
                vec.push(slot);
            }
            Some(index)
        }

        // plant saplings
        for tree in 0..trees {
            let index = fill(self.size, tree);
            let offset = index.component_mul(&spacing);
            let tree = self.position + offset;

            if !turtle.world().occupied(tree).await {
                let sapling = match pop_item(&mut saplings) {
                    Some(slot) => slot,
                    None => break,
                };
                let near = turtle.goto_adjacent(tree).await?;
                turtle.execute(TurtleCommand::Select(sapling)).await;
                turtle.execute(near.place(tree)?).await;
            }
        }

        Some(())
    }

    pub async fn build(&self, turtle: TurtleCommander) -> Option<()> {
        let trees = self.size.x * self.size.y * self.size.z;
        let mut soil_to_lay = Vec::new();
        for tree in 0..trees {
            let index = fill(self.size, tree);
            let offset = index.component_mul(&Vec3::new(2, 32, 2));
            let tree = self.position + offset;
            let soil = tree - Vec3::y();
            if turtle.world().get(soil).await.map_or_else(|| true, |b| b.name.contains("dirt")) {
                soil_to_lay.push(soil);
            }
        }

        for block in soil_to_lay {
            let near = turtle.goto_adjacent(block).await?;
            // TODO: item management
            //turtle.execute(TurtleCommand::Select(soil)).await;
            turtle.execute(near.place(block)?).await;
        }

        Some(())
    }
}

#[serde]
impl Task for TreeFarm {
    fn run(&mut self,turtle:TurtleCommander) -> AbortHandle  {
        let frozen = self.clone();
        tokio::spawn(async move {
            frozen.sweep(turtle).await.unwrap();
        }).abort_handle()
    }

    fn poll(&mut self) -> TaskState  {
        let elapsed = OffsetDateTime::now_utc() - self.last_sweep;
        if elapsed.whole_minutes() <= SWEEP_DELAY {
            return TaskState::Waiting;
        }
        self.last_sweep = OffsetDateTime::now_utc();
        TaskState::Ready(Position::new(self.position, Direction::North)) // request a turtle
    }
}
