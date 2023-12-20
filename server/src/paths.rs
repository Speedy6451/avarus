use std::rc::Rc;

use crate::{
    blocks::{Block, World, Position, Direction, Vec3, WorldReadLock}, turtle::TurtleCommand,
};
use pathfinding::prelude::astar;

pub async fn route_facing(from: Position, to: Vec3, world: &World) -> Option<Vec<Position>> {
    let facing = |p: &Position| {
        let ahead = p.dir.unit() + p.pos;
        let above = Vec3::y() + p.pos;
        let below = -Vec3::y() + p.pos;
        to == ahead || to == below || to == above
    };
    route_to(from, to, facing, world).await
}

pub async fn route(from: Position, to: Position, world: &World) -> Option<Vec<Position>> {
    // attempt at not crashing by looking infinitely into the abyss
    if world.get(to.pos).await
        .is_some_and(|b| difficulty(&b.name).is_none())
    {
        return None;
    }
    route_to(from, to.pos, |p| p == &to, world).await
}

async fn route_to<D>(from: Position, to: Vec3, done: D, world: &World) -> Option<Vec<Position>>
where D: FnMut(&Position) -> bool {
    // lock once, we'll be doing a lot of lookups
    let world = world.clone().lock().await;

    let route = astar(
        &from,
        move |p| next(p, &world),
        |p1| (p1.pos - &to).abs().sum() as u32,
        done,
    )
    .unwrap();
    Some(route.0)
}

fn next(from: &Position, world: &WorldReadLock) -> Vec<(Position, u32)> {
    let mut vec: Vec<(Position, u32)> = Vec::new();

    fn insert(
        vec: &mut Vec<(Position, u32)>,
        point: Vec3,
        orientation: Direction,
        world: &WorldReadLock,
        unknown: Option<u32>,
    ) {
        world
            .locate_at_point(&point.into())
            .map_or(unknown, |b| difficulty(&b.name))
            .map(|d| vec.push((Position::new(point, orientation), d)));
    }

    vec.push((Position::new(from.pos, from.dir.left()), 1));
    vec.push((Position::new(from.pos, from.dir.right()), 1));

    let ahead = from.pos + from.dir.unit();
    insert(&mut vec, ahead, from.dir, world, UNKNOWN);

    let behind = from.pos - from.dir.unit();
    insert(&mut vec, behind, from.dir, world, None);

    let above = from.pos + Vec3::y();
    insert(&mut vec, above, from.dir, world, UNKNOWN);

    let below = from.pos - Vec3::y();
    insert(&mut vec, below, from.dir, world, UNKNOWN);

    vec
}

/// Blocks that are fine to tunnel through
const GARBAGE: [&str; 6] = [
    "minecraft:stone",
    "minecraft:dirt",
    "minecraft:andesite",
    "minecraft:sand",
    "minecraft:gravel",
    "minecraft:sandstone",
];

/// time taken to go through uncharted territory (in turtle. calls)
const UNKNOWN: Option<u32> = Some(2);

// time to go somewhere
pub fn difficulty(name: &str) -> Option<u32> {
    if name == "minecraft:air" {
        return Some(1);
    };
    if GARBAGE.contains(&name) {
        return Some(2);
    };
    None
}
