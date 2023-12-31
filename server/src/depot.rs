use std::sync::Arc;

use tracing::{warn, info, trace};
use tokio::sync::{Mutex, OwnedMutexGuard, Semaphore, OwnedSemaphorePermit};

use crate::{blocks::Position, turtle::TurtleCommander};
use crate::turtle::{TurtleCommand::*, TurtleCommandResponse};


/// List of available depots
///
/// below the specified position is an output chest of infinite capacity
/// ahead of the specified position is a chest of combustibles
#[derive(Clone, Debug)]
pub struct Depots {
    depots: Arc<Mutex<Vec<Arc<Mutex<Position>>>>>,
    depot_semaphore: Arc<Semaphore>,
}

pub struct DepotGuard {
    mutex: OwnedMutexGuard<Position>,
    #[allow(unused)]
    semaphore: OwnedSemaphorePermit, // "dropped in declaration order"
                                     //  - reference chapter 10.8
}

impl DepotGuard {
    fn new(mutex: OwnedMutexGuard<Position>, semaphore: OwnedSemaphorePermit) -> Self { Self { mutex, semaphore } }

    pub fn position(&self) -> &Position {
        &self.mutex
    }
    
}

impl Depots {
    /// Nearest depot to the given position
    pub async fn nearest(&self, pos: Position) -> DepotGuard {
        let permit = self.depot_semaphore.clone().acquire_owned().await.unwrap();
        let mutex = self.depots.lock().await
            .iter().map(|i| i.clone())
            .filter_map(|i| i.try_lock_owned().ok())
            .min_by_key(|d| d.manhattan(pos))
            .map(|d| d);

        DepotGuard::new(mutex.unwrap(), permit)
    }

    pub async fn dock(&self, turtle: TurtleCommander) -> Option<usize> {
        let depot = self.clone().nearest(turtle.pos().await).await;
        trace!("depot at {:?}", depot.position());
        turtle.goto(*depot.position()).await?;

        dump(&turtle).await;
        refuel(&turtle).await;
        
        // This can fail, we don't really care (as long as it executes once)
        turtle.execute(Backward(4)).await;

        drop(depot);

        // lava bucket fix
        for (i, _) in turtle.inventory().await.into_iter().enumerate().filter(|(_,n)| n.is_some()) {
            turtle.execute(Select((i+1) as u32)).await;
            turtle.execute(DropDown(64)).await;
        }


        Some(turtle.fuel())
    }

    pub async fn add(&self, pos: Position) {
        info!("new depot at {pos:?}");
        self.depots.lock().await.push(Arc::new(Mutex::new(pos)));
        self.depot_semaphore.add_permits(1);
    }

    pub fn from_vec(vec: Vec<Position>) -> Self {
        let mut depots = Vec::new();
        for depot in vec {
            depots.push(Arc::new(Mutex::new(depot)));
        }
        let permits = depots.len();
        Depots { depots: Arc::new(Mutex::new(depots)),
            depot_semaphore: Arc::new(Semaphore::new(permits))
        }
    }

    pub async fn to_vec(self) -> Vec<Position> {
        let mut depots = Vec::new();
        for depot in self.depots.lock().await.iter() {
            depots.push(*depot.lock().await)
        }
        depots
    }
}

pub async fn dump(turtle: &TurtleCommander) {
    for (i, _) in turtle.inventory().await.into_iter().enumerate().filter(|(_,n)| n.is_some()) {
        turtle.execute(Select((i+1) as u32)).await;
        turtle.execute(DropDown(64)).await;
    }
}


pub async fn refuel(turtle: &TurtleCommander) {
    turtle.execute(Select(1)).await;
    let limit = turtle.fuel_limit();
    while turtle.fuel() + 1000 < limit {
        turtle.execute(SuckFront(64)).await;
        let re = turtle.execute(Refuel).await;
        turtle.execute(DropDown(64)).await;
        if let TurtleCommandResponse::Failure = re.ret {
            // partial refuel, good enough
            warn!("only received {} fuel", turtle.fuel());
            if turtle.fuel() > 1500 {
                break;
            } else {
                turtle.execute(Wait(15)).await;
            }
        }
    }
}
