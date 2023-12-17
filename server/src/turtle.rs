use std::rc::Rc;

use pathfinding::prelude::astar;
use crate::{blocks::{World, Block}, Position, Direction};

use super::Vec3;

pub fn route(from: Position, to: Position, world: &World) -> Vec<Position> {
    let route = astar(&from, move |p| {next(p, world)} , |p1| {(p1.0 - &to.0).abs().sum() as u32}, |p| {p == &to}).unwrap();
    route.0
}

fn next(from: &Position, world: &World) -> Vec<(Position, u32)> {
    let mut vec: Vec<(Position, u32)> = Vec::new();
    vec.push(((from.0, from.1.left()),1));
    vec.push(((from.0, from.1.right()),1));

    fn insert(vec: &mut Vec<(Position, u32)>, point: Vec3, orientation: Direction, world: &World) {
        world.locate_at_point(&point.into())
            .map_or(Some(UNKNOWN), |b| difficulty(&b.name))
            .map(|d| vec.push(((point, orientation), d)));
    }

    let ahead = from.0 + from.1.unit();
    insert(&mut vec, ahead, from.1, world);

    let behind = from.0 - from.1.unit();
    insert(&mut vec, behind, from.1, world);
    
    let above = from.0 + Vec3::y();
    insert(&mut vec, above, from.1, world);

    let below = from.0 - Vec3::y();
    insert(&mut vec, below, from.1, world);

    vec
}

/// Blocks that are fine to tunnel through
const GARBAGE: [&str; 3] = [
    "minecraft:stone",
    "minecraft:dirt",
    "minecraft:andesite",
];

/// time taken to go through uncharted territory (in turtle. calls)
const UNKNOWN: u32 = 2;

// time to go somewhere
fn difficulty(name: &str) -> Option<u32> {
    if name == "minecraft:air" { return Some(1) };
    if GARBAGE.contains(&name) { return Some(2)};
    None
}
