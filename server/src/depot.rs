use std::sync::Arc;

use log::{warn, info, trace};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::{blocks::Position, turtle::TurtleCommander};
use crate::turtle::{TurtleCommand::*, TurtleCommandResponse};


/// List of available depots
///
/// below the specified position is an output chest of infinite capacity
/// ahead of the specified position is a chest of combustibles
#[derive(Clone)]
pub struct Depots {
    depots: Arc<Mutex<Vec<Arc<Mutex<Position>>>>>
}

impl Depots {
    /// Nearest depot to the given position
    pub async fn nearest(&self, pos: Position) -> Option<OwnedMutexGuard<Position>> {
        self.depots.lock().await
            .iter().map(|i| i.clone())
            .filter_map(|i| i.try_lock_owned().ok())
            .min_by_key(|d| d.manhattan(pos))
            .map(|d| d)
    }

    pub async fn dock(&self, turtle: TurtleCommander) -> Option<usize> {
        let depot = self.clone().nearest(turtle.pos().await).await?;
        trace!("depot at {depot:?}");
        turtle.goto(*depot).await?;

        // dump inventory
        for i in 1..=16 {
            turtle.execute(Select(i)).await;
            turtle.execute(DropDown(64)).await;
        }

        // refuel
        turtle.execute(Select(1)).await;
        let limit = turtle.fuel_limit();
        while turtle.fuel() + 1000 < limit {
            turtle.execute(SuckFront(64)).await;
            let re = turtle.execute(Refuel).await;
            turtle.execute(DropDown(64)).await;
            if let TurtleCommandResponse::Failure = re.ret {
                // partial refuel, good enough
                warn!("only received {} fuel", turtle.fuel());
                if turtle.fuel() > 5000 {
                    break;
                } else {
                    turtle.execute(Wait(15)).await;
                }
            }
        }

        // lava bucket fix
        for i in 1..=16 {
            turtle.execute(Select(i)).await;
            turtle.execute(DropDown(64)).await;
        }
        
        turtle.execute(Backward(1)).await;

        drop(depot); // assumes that the turtle will very quickly leave

        Some(turtle.fuel())
    }

    pub async fn add(&self, pos: Position) {
        info!("new depot at {pos:?}");
        self.depots.lock().await.push(Arc::new(Mutex::new(pos)))
    }

    pub fn from_vec(vec: Vec<Position>) -> Self {
        let mut depots = Vec::new();
        for depot in vec {
            depots.push(Arc::new(Mutex::new(depot)));
        }
        Depots { depots: Arc::new(Mutex::new(depots)) }
    }

    pub async fn to_vec(self) -> Vec<Position> {
        let mut depots = Vec::new();
        for depot in self.depots.lock().await.iter() {
            depots.push(*depot.lock().await)
        }
        depots
    }
}
