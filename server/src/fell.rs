use serde::{Serialize, Deserialize};
use time::OffsetDateTime;
use tokio::task::JoinHandle;
use typetag::serde;

use crate::{blocks::{Vec3, Position, Direction}, turtle::TurtleCommander, tasks::Task, depot::Depots, mine::fill};

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
const SWEEP_DELAY: usize = 16;

#[derive(Serialize, Deserialize, Clone)]
struct TreeFarm {
    position: Vec3,
    size: Vec3,
    last_sweep: OffsetDateTime,
}

impl TreeFarm {
    pub async fn sweep(&self, turtle: TurtleCommander) -> Option<()> {
        let trees = self.size.x * self.size.y * self.size.z;
        turtle.dock().await;
        for tree in 0..trees {
            let index = fill(self.size, tree);
            let offset = index.component_mul(&Vec3::new(2, 32, 2));
            let tree = self.position + offset;
            fell_tree(turtle.clone(), tree).await?;
        }

        Some(())
    }

    pub async fn build(&self, turtle: TurtleCommander) -> Option<()> {
        let trees = self.size.x * self.size.y * self.size.z;
        turtle.dock().await;
        for tree in 0..trees {
            let index = fill(self.size, tree);
            let offset = index.component_mul(&Vec3::new(2, 32, 2));
            let tree = self.position + offset;
            // TODO: item management
        }

        Some(())
    }
}

#[serde]
impl Task for TreeFarm {
    fn run(&mut self,turtle:TurtleCommander) -> JoinHandle<()>  {
        let frozen = self.clone();
        tokio::spawn(async move {
            frozen.sweep(turtle).await.unwrap();
        })
    }

    fn poll(&mut self) -> Option<Position>  {
        let elapsed = OffsetDateTime::now_utc() - self.last_sweep;
        if elapsed.whole_minutes() <= 16 {
            return None;
        }
        self.last_sweep = OffsetDateTime::now_utc();
        Some(Position::new(self.position, Direction::North)) // request a turtle
    }
}
