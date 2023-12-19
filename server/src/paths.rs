use std::rc::Rc;

use crate::{
    blocks::{Block, World, Position, Direction, Vec3, WorldReadLock},
};
use pathfinding::prelude::astar;


pub async fn route(from: Position, to: Position, world: &World) -> Option<Vec<Position>> {
    // lock once, we'll be doing a lot of lookups
    let world = world.clone().lock().await;

    // attempt at not crashing by looking infinitely into the abyss
    if world
        .locate_at_point(&to.pos.into())
        .is_some_and(|b| difficulty(&b.name).is_none())
    {
        return None;
    }
    let route = astar(
        &from,
        move |p| next(p, &world),
        |p1| (p1.pos - &to.pos).abs().sum() as u32,
        |p| p == &to,
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

    //let behind = from.pos - from.dir.unit();
    //insert(&mut vec, behind, from.dir, world, None);

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
fn difficulty(name: &str) -> Option<u32> {
    if name == "minecraft:air" {
        return Some(1);
    };
    if GARBAGE.contains(&name) {
        return Some(2);
    };
    None
}
