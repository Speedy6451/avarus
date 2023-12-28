use crate::{
    blocks::{SharedWorld, Position, Direction, Vec3, World, nearest, Block},
};
use rstar::{AABB, Envelope};
use tokio::task::spawn_blocking;
use tracing::{trace, error};
use pathfinding::prelude::astar;

const LOOKUP_LIMIT: usize = 100_000_000;

#[tracing::instrument(skip(world))]
pub async fn route_facing(from: Position, to: Vec3, world: &SharedWorld) -> Option<Vec<Position>> {
    let facing = move |p: &Position| {
        let ahead = p.dir.unit() + p.pos;
        let above = Vec3::y() + p.pos;
        let below = -Vec3::y() + p.pos;
        to == ahead || to == below || to == above
    };
    route_to(from, to, facing, world).await
}

#[tracing::instrument(skip(world))]
pub async fn route(from: Position, to: Position, world: &SharedWorld) -> Option<Vec<Position>> {
    trace!("routing from {from:?} to {to:?}");
    // attempt at not crashing by looking infinitely into the abyss
    if world.get(to.pos).await
        .is_some_and(|b| difficulty(&b.name).is_none())
    {
        return None;
    }
    route_to(from, to.pos, move |p| p == &to, world).await
}

async fn route_to<D>(from: Position, to: Vec3, mut done: D, world: &SharedWorld) -> Option<Vec<Position>>
where D: FnMut(&Position) -> bool + Send + 'static {
    // lock once, we'll be doing a lot of lookups
    let world = world.clone().lock().await;

    let mut limit = LOOKUP_LIMIT;

    let route = 
        spawn_blocking( move ||
        astar(
        &from,
        move |p| next(p, &world),
        |p1| (p1.pos - &to).abs().sum() as u32,
        |p| {
            limit -= 1;
            if limit == 0 {
                return true
            } else {
                done(p)
            }
        },
    )).await.unwrap()?;

    trace!("scanned {} states", LOOKUP_LIMIT-limit);
    if limit != 0 {
        Some(route.0)
    } else {
        error!("pathfinding timed out");
        None
    }
}

fn next(from: &Position, world: &World) -> Vec<(Position, u32)> {
    let mut vec: Vec<(Position, u32)> = Vec::new();

    fn insert(
        vec: &mut Vec<(Position, u32)>,
        point: Vec3,
        orientation: Direction,
        world: &World,
        unknown: Option<u32>,
    ) {
        world
            .get(point)
            .map_or(unknown, |b| difficulty(&b.name))
            .map(|d| vec.push((Position::new(point, orientation), d)));
    }

    vec.push((Position::new(from.pos, from.dir.left()), 1));
    vec.push((Position::new(from.pos, from.dir.right()), 1));

    let ahead = from.pos + from.dir.unit();
    insert(&mut vec, ahead, from.dir, world, UNKNOWN);

    //let behind = from.pos - from.dir.unit();
    //insert(&mut vec, behind, from.dir, world, None);

    let above = from.pos + Vec3::y();
    insert(&mut vec, above, from.dir, world, UNKNOWN);

    let below = from.pos - Vec3::y();
    insert(&mut vec, below, from.dir, world, UNKNOWN);

    vec
}

/// Blocks that you can go through without a pickaxe
pub const TRANSPARENT: [&str; 3] = [
    "minecraft:air",
    "minecraft:water",
    "minecraft:lava",
];

/// Blocks that are fine to tunnel through
const GARBAGE: [&str; 15] = [
    "minecraft:stone",
    "minecraft:dirt",
    "minecraft:andesite",
    "minecraft:sand",
    "minecraft:gravel",
    "minecraft:sandstone",
    "minecraft:deepslate",
    "twigs:rhyolite",
    "minecraft:spruce_leaves",
    "minecraft:oak_leaves",
    "traverse:fir_leaves",
    "terrestria:hemlock_leaves",
    "wilderwild:brown_shelf_fungus",
    "wilderwild:red_shelf_fungus",
    "minecraft:snow",
];

/// time taken to go through uncharted territory (in turtle. calls)
const UNKNOWN: Option<u32> = Some(2);

// time to go somewhere
pub fn difficulty(name: &str) -> Option<u32> {
    if TRANSPARENT.contains(&name) {
        return Some(1);
    };
    if GARBAGE.contains(&name) {
        return Some(2);
    };
    None
}
