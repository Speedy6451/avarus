use log::{info, warn};

use crate::{blocks::{Position, Vec3}, turtle::{TurtleCommand, TurtleCommander, TurtleCommandResponse, InventorySlot}, paths::TRANSPARENT};
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

pub async fn mine(turtle: TurtleCommander, pos: Vec3, fuel: Position, storage: Position) -> Option<()> {
    let chunk = Vec3::new(4,4,4);
    let volume = chunk.x * chunk.y * chunk.z;
    let mut pos = pos;
    let mut valuables = Vec::new();

    async fn refuel_needed(turtle: &TurtleCommander, volume: i32, fuel: Position) -> Option<()> {
        Some(if (turtle.fuel() as f64) < (2 * volume + (fuel.pos-turtle.pos().await.pos).abs().sum()) as f64 * 1.8 {
            let name = turtle.name().to_str();
            info!("{name}: refueling");
            turtle.goto(fuel).await?;
            info!("{name}: docked");
            refuel(turtle.clone()).await;
        })
    }

    loop {
        refuel_needed(&turtle, volume, fuel).await?;

        mine_chunk(turtle.clone(), pos, chunk).await?;

        valuables.append(&mut near_valuables(&turtle, pos, chunk).await);

        while let Some(block) = valuables.pop() {
            if turtle.world().get(block).await.is_none() {
                continue;
            }
            let near = turtle.goto_adjacent(block).await?;
            turtle.execute(near.dig(block)?).await;
            observe(turtle.clone(), block).await;
            valuables.append(&mut near_valuables(&turtle, near.pos, Vec3::new(2,2,2)).await);

            refuel_needed(&turtle, volume, fuel).await?;
        }

        if dump_filter(turtle.clone(), |i| USELESS.iter().any(|u| **u == i.name)).await > 12 {
            info!("storage rtb");
            turtle.goto(storage).await?;
            dump(turtle.clone()).await;
            // while we're here
            turtle.goto(fuel).await?;
            refuel(turtle.clone()).await;
        }

        pos += Vec3::z() * chunk.z;
    }
}

async fn near_valuables(turtle: &TurtleCommander, pos: Vec3, chunk: Vec3) -> Vec<Vec3> {
    turtle.world().lock().await
        .locate_within_distance(pos.into(), chunk.map(|n| n.pow(2)).sum()) 
        .filter(|n| n.name != "minecraft:air")
        .filter(|n| VALUABLE.iter().any(|v| n.name.contains(v)))
        .map(|b|b.pos).collect()
}

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
    for i in 1..=16 {
        if let TurtleCommandResponse::Item(item) = turtle.execute(ItemInfo(i)).await.ret {
            if filter(item) {
                turtle.execute(Select(i)).await;
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
