use std::rc::Rc;

use pathfinding::prelude::astar;
use crate::{blocks::{World, Block}, Position};

use super::Vec3;

pub fn route(from: Position, to: Position, world: World) -> Vec<Position> {
    let world = Rc::new(world);
    let route = astar(&from, move |p| {next(p, world.clone())} , |p1| {(p1.0 - &to.0).abs().sum() as u32}, |p| {p == &to}).unwrap();
    route.0
}

fn next(from: &Position, world: Rc<World>) -> Vec<(Position, u32)> {
    let mut vec: Vec<(Position, u32)> = Vec::new();
    vec.push(((from.0, from.1.left()),1));
    vec.push(((from.0, from.1.right()),1));
    let ahead = from.0 + from.1.unit();

    let empty: Block = Block {
        name: String::from("minecraft:air"),
        pos: Vec3::zeros(),
    };

    let block_ahead = world.locate_at_point(&ahead.into()).unwrap_or(&empty);
    difficulty(&block_ahead.name).map(|d| vec.push(((ahead, from.1), d)));

    let behind = from.0 - from.1.unit();
    let block_behind = world.locate_at_point(&behind.into()).unwrap_or(&empty);
    difficulty(&block_behind.name).map(|d| vec.push(((behind, from.1), d)));
    
    let above = from.0 + Vec3::y();
    let block_above = world.locate_at_point(&above.into()).unwrap_or(&empty);
    difficulty(&block_above.name).map(|d| vec.push(((above, from.1), d)));

    let below = from.0 - Vec3::y();
    let block_below = world.locate_at_point(&below.into()).unwrap_or(&empty);
    difficulty(&block_below.name).map(|d| vec.push(((below, from.1), d)));

    vec
}

/// Blocks that are fine to tunnel through
const GARBAGE: [&str; 3] = [
    "minecraft:stone",
    "minecraft:dirt",
    "minecraft:andesite",
];

// time to go somewhere
fn difficulty(name: &str) -> Option<u32> {
    if name == "minecraft:air" { return Some(1) };
    if GARBAGE.contains(&name) { return Some(2)};
    None
}
