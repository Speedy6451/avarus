use std::sync::{Arc, atomic::{AtomicUsize, Ordering, AtomicI32}};

use tracing::{info, warn, error, instrument};
use serde::{Serialize, Deserialize};
use tokio::{task::{JoinHandle, AbortHandle}, sync::RwLock};
use typetag::serde;

use crate::{blocks::{Position, Vec3, Direction}, turtle::{TurtleCommand, TurtleCommander, TurtleCommandResponse, InventorySlot}, paths::TRANSPARENT, tasks::{Task, TaskState}};
use TurtleCommand::*;

/// Things to leave in the field (not worth fuel)
const USELESS: [&str; 5] = [
    "minecraft:dirt",
    "minecraft:gravel",
    "minecraft:cobblestone",
    "minecraft:cobbled_deepslate",
    "minecraft:rhyolite",
    //"minecraft:andesite", // TODO: Reach 2k
];

/// Things that are desirable
const VALUABLE: [&str; 1] = [
    "ore",
];

pub async fn mine(turtle: TurtleCommander, pos: Vec3, chunk: Vec3) -> Option<()> {
    let mut pos = pos;

    loop {
        mine_chunk_and_sweep(turtle.clone(), pos, chunk).await?;

        pos += Vec3::z() * chunk.z;
    }
}

#[instrument]
pub async fn mine_chunk_and_sweep(turtle: TurtleCommander, pos: Vec3, chunk: Vec3) -> Option<()> {
    let volume = chunk.x * chunk.y * chunk.z;
    let mut valuables = Vec::new();

    async fn refuel_needed(turtle: &TurtleCommander, volume: i32) {
        if (turtle.fuel() as i32) < 2 * volume + 4000 {
            turtle.dock().await;
        }
    }

    if dump_filter(turtle.clone(), |i| USELESS.iter().any(|u| **u == i.name)).await > 12 {
        info!("storage rtb");
        turtle.dock().await;
    }

    devore(&turtle).await;

    refuel_needed(&turtle, volume).await;

    mine_chunk(turtle.clone(), pos, chunk).await?;

    valuables.append(&mut near_valuables(&turtle, pos, chunk).await);

    while let Some(block) = valuables.pop() {
        refuel_needed(&turtle, volume).await;

        if turtle.world().garbage(block).await {
            continue;
        }
        let near = turtle.goto_adjacent(block).await?;
        turtle.execute(near.dig(block)?).await;
        observe(turtle.clone(), block).await;
        valuables.append(&mut near_valuables(&turtle, near.pos, Vec3::new(2,2,2)).await);
    }

    Some(())
}

/// Send mined turtles to the nearest depot
async fn devore(turtle: &TurtleCommander) {
    let turtles: Vec<u32> = turtle.inventory().await.into_iter().enumerate()
        .filter(|(_,b)| b.as_ref().is_some_and(|b| b.name.contains("turtle")))
        .map(|(i,_)| (i + 1) as u32).collect();

    if turtles.is_empty() {
        return;
    }

    let depot = turtle.get_depot().await;

    for i in turtles {
        let position = depot.position();

        let staging = position.pos - position.dir.unit();

        turtle.goto(Position::new(staging, position.dir)).await;
        warn!("devoring {i}");
        turtle.execute(Select(i)).await;
        turtle.execute(Place).await;
        turtle.execute(CycleFront).await;
        loop {
            let ret = turtle.execute(Wait(3)).await;
            // this won't do well with dead (energy-lacking) turtles, perhaps obtaining 
            // a new depot (lock) for every turtle is more consistent
            //
            // alternatively, figure out label parsing (names with spaces) 
            // and issue a command to the child turtle
            if !ret.ahead.contains("turtle") {
                break;
            }
            warn!("devored turtle still inactive");
        }
    }
}

async fn near_valuables(turtle: &TurtleCommander, pos: Vec3, chunk: Vec3) -> Vec<Vec3> {
    let scan = (0..(chunk*2).product()).map(|n| fill(chunk * 2, n) - chunk/2);
        
    let world = turtle.world().lock().await;
    scan.map(|n| world.get(n + pos))
        .filter_map(|f| f)
        .filter(|n| n.name != "minecraft:air")
        .filter(|n| VALUABLE.iter().any(|v| n.name.contains(v)))
        .map(|b|b.pos).collect()
}

#[instrument]
pub async fn mine_chunk(turtle: TurtleCommander, pos: Vec3, chunk: Vec3) -> Option<()> {
    let turtle = turtle.clone();
    let volume = chunk.x * chunk.y * chunk.z;

    for n in (0..volume).map(|n| fill(chunk, n) + pos) {
        if turtle.world().get(n).await.is_some_and(|b| TRANSPARENT.contains(&b.name.as_str())) {
            continue;
        }

        let near = turtle.goto_adjacent(n).await?;

        turtle.execute(near.dig(n)?).await;
        
    }
    Some(())
}

async fn refuel(turtle: TurtleCommander) {
    turtle.execute(Select(16)).await;
    turtle.execute(DropUp(64)).await;
    let limit = turtle.fuel_limit();
    while turtle.fuel() < limit {
        turtle.execute(SuckFront(64)).await;
        let re = turtle.execute(Refuel).await;
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
    turtle.execute(DropFront(64)).await;
}

async fn dump(turtle: TurtleCommander) {
    for i in 1..=16 {
        turtle.execute(Select(i)).await;
        turtle.execute(DropFront(64)).await;
    }
}

/// Dump all items that match the predicate
/// Returns the number of slots still full after the operation
async fn dump_filter<F>(turtle: TurtleCommander, mut filter: F) -> u32
where F: FnMut(InventorySlot) -> bool {
    let mut counter = 0;
    for (i, slot) in turtle.inventory().await.into_iter().enumerate() {
        if let Some(item) = slot {
            if filter(item) {
                turtle.execute(Select(i as u32)).await;
                turtle.execute(DropFront(64)).await;
            } else {
                counter += 1;
            }
        }
    }
    counter
}

/// zig from 0 to x and back, stopping on each end
fn step(n: i32, x: i32) -> i32 {
    let half = n%x;
    let full = n%(2*x);
    if full > x - 1 {
        x - half - 1
    } else {
        full
    }
}

/// generates a sequence of adjacent positions within a volume
pub fn fill(scale: Vec3, n: i32) -> Vec3 {
    assert!(n < scale.x * scale.y * scale.z);
    Vec3::new(
        step(n,scale.x),
        step(n/scale.x, scale.y),
        step(n/scale.x/scale.y, scale.z)
    )
}

/// Looks at all the blocks around the given pos
/// destructive
async fn observe(turtle: TurtleCommander, pos: Vec3) -> Option<()> {
    let adjacent = [
        pos, 
        pos + Vec3::y(),
        pos + Vec3::x(),
        pos + Vec3::z(),
        pos - Vec3::x(),
        pos - Vec3::z(),
        pos - Vec3::y(),
    ];

    for pos in adjacent {
        if turtle.world().get(pos).await.is_none() {
            turtle.goto_adjacent(pos).await?;
        }
        
    }

    Some(())
}

#[derive(Serialize, Deserialize,Clone)]
pub struct Mine {
    pos: Vec3,
    chunk: Vec3,
    #[serde(skip_deserializing)]
    miners: usize, // Default is false
}

impl Mine {
    pub fn new(pos: Vec3, chunk: Vec3) -> Self { Self { pos, chunk, miners: 0 } }
}

#[serde]
impl Task for Mine {
    fn run(&mut self,turtle:TurtleCommander) -> AbortHandle {
        self.miners += 1;
        let frozen = self.clone();
        tokio::spawn(async move {
            mine(turtle,frozen.pos, frozen.chunk).await.unwrap();
        }).abort_handle()
        // TODO: mutability after spawn
    }

    fn poll(&mut self) -> TaskState {
        if self.miners < 1 {
            return TaskState::Ready(Position::new(self.pos, Direction::North));
        }
        TaskState::Waiting
    }
}

const MAX_MINERS: usize = 42;

#[derive(Serialize, Deserialize,Clone)]
pub struct Quarry {
    pos: Vec3,
    size: Vec3,
    #[serde(skip_deserializing)]
    miners: Arc<AtomicUsize>,
    progress: ChunkedTask,
}

impl Quarry {
    pub fn new(lower: Vec3, upper: Vec3) -> Self {
        let size = upper - lower;

        let max_chunk = Vec3::new(4,4,4);
        let chunks = size.component_div(&max_chunk);

        Self { 
            pos: lower, 
            size, 
            miners: Arc::new(AtomicUsize::new(0)),
            progress: ChunkedTask::new(chunks.product())
        }
    }

    pub fn chunk(pos: Vec3) -> Self {
        let base = pos - pos.map(|n| n%16);
        Self::new(base, base+Vec3::new(16,16,16))
    }
}

#[serde]
impl Task for Quarry {
    #[instrument(skip(self))]
    fn run(&mut self,turtle:TurtleCommander) -> AbortHandle {
        let owned = self.clone();
        tokio::spawn(async move {
            let chunk = owned.progress.next_chunk().await;

            if let None = chunk {
                error!("scheduled quarry out of range");
                return;
            }
            let chunk = chunk.unwrap();

            info!("#{} doing chunk {chunk}", turtle.name().to_str());

            let max_chunk = Vec3::new(4,4,4);
            let e = owned.size.component_div(&max_chunk);

            let rel_pos = fill(e, chunk).component_mul(&max_chunk);
            let abs_pos = rel_pos
                + owned.pos;
            if let None = mine_chunk_and_sweep(turtle, abs_pos, max_chunk).await {
                error!("mining at {abs_pos} failed");
                owned.progress.cancel(chunk).await;
            } else {
                owned.progress.mark_done(chunk).await;
            }
            owned.miners.fetch_sub(1, Ordering::AcqRel);
        }).abort_handle()
    }

    fn poll(&mut self) -> TaskState {
        if self.progress.done() {
            return TaskState::Complete;
        }

        if self.progress.allocated() {
            return TaskState::Waiting;
        }

        let only = self.miners.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            if n < MAX_MINERS {
                Some(n+1)
            }else {
                None
            }
        }).is_ok();

        if only {
            // This is approximate as we have to go to a depot anyway
            return TaskState::Ready(Position::new(self.pos, Direction::North));
        }
        TaskState::Waiting
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct ChunkedTask {
    confirmed: Arc<AtomicI32>,
    #[serde(skip_deserializing)]
    head: Arc<AtomicI32>, // highest active chunk
    #[serde(skip)]
    canceled: Arc<RwLock<Vec<i32>>>, // must remain sorted
    max: i32,
}

impl ChunkedTask {
    fn new(parts: i32) -> Self { 
        Self {
            confirmed: Default::default(),
            head: Default::default(),
            canceled: Default::default(),
            max: parts,
        } 
    }

    fn done(&self) -> bool {
        let backstop = self.confirmed.load(Ordering::SeqCst);
        backstop + 1 >= self.max
    }

    fn allocated(&self) -> bool {
        let front = self.head.load(Ordering::SeqCst);
        front + 1 >= self.max
    }

    async fn next_chunk(&self) -> Option<i32> {
        let mut cancelled = self.canceled.clone().write_owned().await;

        if let Some(chunk) = cancelled.pop() {
            return Some(chunk);
        }

        loop { // update head (from a save)
            let minimum = self.confirmed.load(Ordering::SeqCst);
            let head = self.head.load(Ordering::SeqCst);
            if let Ok(_) = self.head.compare_exchange(head, minimum.max(head), Ordering::AcqRel, Ordering::SeqCst) {
                break;
            }
        }

        let head = self.head.fetch_add(1, Ordering::AcqRel);

        if head < self.max {
            Some(head)
        } else {
            None
        }
    }

    async fn mark_done(&self, chunk: i32) {
        let canceled = self.canceled.read().await;

        let min = match canceled.iter().min() {
            None => true,
            Some(minima) => chunk < *minima,
        };

        if min {
            loop {
                let curr = self.confirmed.load(Ordering::SeqCst);
                if let Ok(_) = self.confirmed.compare_exchange(curr, curr.max(chunk), Ordering::AcqRel, Ordering::SeqCst) {
                    break;
                }
            }
        }
    }

    async fn cancel(&self, chunk: i32) {
        let mut in_flight = self.canceled.write().await;
        let max = self.head.load(Ordering::SeqCst);
        if chunk < max {
            in_flight.push(chunk);
            in_flight.sort_unstable();
        }
        else {
            error!("attempted to cancel a job that hasn't happened yet");
        }
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn linear() {
        let tracker = ChunkedTask::new(5);
        assert_eq!(tracker.next_chunk().await, Some(0));
        tracker.mark_done(0).await;
        assert_eq!(tracker.next_chunk().await, Some(1));
        tracker.mark_done(1).await;
        assert_eq!(tracker.next_chunk().await, Some(2));
        assert_eq!(tracker.done(), false);
        tracker.mark_done(2).await;
        assert_eq!(tracker.next_chunk().await, Some(3));
        tracker.mark_done(3).await;
        assert_eq!(tracker.next_chunk().await, Some(4));
        assert_eq!(tracker.next_chunk().await, None);
        assert_eq!(tracker.done(), false);
        assert_eq!(tracker.next_chunk().await, None);
        assert_eq!(tracker.allocated(), true);
        tracker.mark_done(4).await;
        assert_eq!(tracker.done(), true);
    }

    #[tokio::test]
    async fn cancel_replay() {
        let tracker = ChunkedTask::new(5);
        assert_eq!(tracker.next_chunk().await, Some(0));
        tracker.mark_done(0).await;
        assert_eq!(tracker.next_chunk().await, Some(1));
        tracker.mark_done(1).await;
        tracker.cancel(2).await;
        assert_eq!(tracker.next_chunk().await, Some(2));
        assert_eq!(tracker.done(), false);
        tracker.mark_done(2).await;
        assert_eq!(tracker.next_chunk().await, Some(3));
        tracker.mark_done(3).await;
        tracker.cancel(2).await;
        assert_eq!(tracker.next_chunk().await, Some(2));
        assert_eq!(tracker.next_chunk().await, Some(4));
        tracker.cancel(1).await;
        assert_eq!(tracker.next_chunk().await, Some(1));
        assert_eq!(tracker.done(), false);
        assert_eq!(tracker.next_chunk().await, None);
        assert_eq!(tracker.allocated(), true);
        tracker.mark_done(4).await;
        assert_eq!(tracker.done(), true);
    }

    #[tokio::test]
    async fn cancel_unexisting() {
        let tracker = ChunkedTask::new(5);
        assert_eq!(tracker.next_chunk().await, Some(0));
        assert_eq!(tracker.next_chunk().await, Some(1));
        assert_eq!(tracker.next_chunk().await, Some(2));
        tracker.cancel(2).await;
        tracker.cancel(5).await;
        tracker.cancel(3).await;
        assert_eq!(tracker.next_chunk().await, Some(2));
        assert_eq!(tracker.next_chunk().await, Some(3));
        assert_eq!(tracker.next_chunk().await, Some(4));
        assert_eq!(tracker.next_chunk().await, None);
    }
}
