use std::{sync::Arc, ops::Sub};

use anyhow::Ok;
use nalgebra::Vector3;
use rstar::{PointDistance, RTree, RTreeObject, AABB, Envelope};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, OwnedRwLockReadGuard};
use memoize::memoize;

use crate::{turtle::TurtleCommand, paths::{self, TRANSPARENT}};

const CHUNK_SIZE: usize = 16;
const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;
const CHUNK_VEC: Vec3  = Vec3::new(CHUNK_SIZE as i32, CHUNK_SIZE as i32, CHUNK_SIZE as i32);

pub struct World(RTree<Chunk>);

impl World {
    pub fn get(&self, block: Vec3) -> Option<Block> {
        let chunk = block.component_div(&CHUNK_VEC);
        let chunk = self.get_chunk(chunk)?;
        chunk.get(block)
    }

    pub fn set(&mut self, block: Block) -> Option<Block> {
        let chunk = block.pos.component_div(&CHUNK_VEC);
        let chunk = self.get_chunk(chunk)?;
        chunk.set(block)
    }

    fn get_chunk(&self, block: Vec3) -> &Chunk {
        let block = block.component_div(&CHUNK_VEC);
        if let Some(chunk) = self.0.locate_at_point(&block.into()) {
            return chunk;
        }
        self.0.insert(Chunk::new(block));
        &Chunk::new(block)
    }

    pub fn get_bulk<const COUNT:usize>(&self, blocks: [Vec3;COUNT]) -> [Option<&Block>;COUNT] {
        let mut chunk: Option<&Chunk> = None;

        blocks.iter().map(|b|{
            if !chunk.is_some_and(|c| c.contains(b)) {
                chunk = Some(self.get_chunk(b));
            }
            chunk.unwrap().get(b)
        }).collect()
    }
    
}

#[derive(Clone)]
pub struct SharedWorld {
    state: Arc<RwLock<World>>, // interior mutability to get around the 
                              // questionable architecture of this project
}

impl SharedWorld {
    pub fn new() -> Self { Self { state: Arc::new(RwLock::new(RTree::new())) } }
    pub fn from_world(tree: World) -> Self { Self { state: Arc::new(RwLock::new(tree)) } }

    pub async fn get(&self, block: Vec3) -> Option<Block> {
        self.state.read().await.get(block)
    }

    pub async fn set(&self, block: Block) {
        self.state.write().await.set(block);
    }

    /// Returns true if a known non-traversable block exists at the point
    pub async fn occupied(&self, block: Vec3) -> bool {
        self.get(block).await.is_some_and(|b| !TRANSPARENT.contains(&b.name.as_str()))
    }

    /// Returns true if a "garbage" block exists at the given point which you are free to destroy
    pub async fn garbage(&self, block: Vec3) -> bool {
        self.get(block).await.is_some_and(|b| paths::difficulty(&b.name).is_some())
    }

    pub async fn lock(self) -> OwnedRwLockReadGuard<World> {
        self.state.read_owned().await
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Block {
    pub name: String,
    pub pos: Vec3,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Chunk {
    pos: Vec3, /// position in chunk coordinates (world/16)
    data: [[[Option<Block>;CHUNK_SIZE];CHUNK_SIZE];CHUNK_SIZE]
}

impl Chunk {
    fn new(pos: Vec3) -> Self {
        Self {
            pos,
            data:[[[None;CHUNK_SIZE];CHUNK_SIZE];CHUNK_SIZE]
        }
    }

    fn set(&mut self, pos: Block) -> Result<(), ()> {
        let chunk = self.pos.component_mul(&CHUNK_VEC);
        let local = pos.pos - chunk;
        if !self.contains(&pos) {
            return Err(());
        }

        self.data[local.x][local.y][local.z] = pos;

        Ok(())
    }

    fn get(&self, pos: Position) -> Option<&Block> {
        let chunk = self.pos.component_mul(&CHUNK_VEC);
        let local = pos.pos - chunk;
        if !self.contains(&pos) {
            return None;
        }

        Ok(self.data[local.x][local.y][local.z])
    }

    fn contains(&self, pos:&Position) -> bool {
        let chunk = self.pos.component_mul(&CHUNK_VEC);
        let local = pos.pos - chunk;
        AABB::from_corners(chunk, chunk+CHUNK_VEC).contains_point(&local)
    }
}

impl RTreeObject for Chunk {
    type Envelope = AABB<[i32; 3]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.pos.into())
    }
}

impl PointDistance for Chunk {
    fn distance_2(&self, point: &[i32; 3]) -> i32 {
        (self.pos - Vec3::from(*point)).abs().sum()
    }
}

pub type Vec3 = Vector3<i32>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Position {
    pub pos: Vec3,
    pub dir: Direction,
}

impl Position {
    pub fn new(pos: Vec3, dir: Direction) -> Self { Self { pos, dir } }

    /// Get a turtle command to map two adjacent positions
    pub fn difference(self, to: Position) -> Option<TurtleCommand> {
        use crate::turtle::TurtleCommand::*;

        if self.pos == to.pos {
            if to.dir == self.dir.left() {
                Some(Left)
            } else if to.dir == self.dir.right() {
                Some(Right)
            } else {
                None
            }
        } else if to.dir == self.dir {
            if to.pos == self.pos + self.dir.unit() {
                Some(Forward(1))
            } else if to.pos == self.pos - self.dir.unit() {
                Some(Backward(1))
            } else if to.pos == self.pos + Vec3::y() {
                Some(Up(1))
            } else if to.pos == self.pos - Vec3::y() {
                Some(Down(1))
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Command to place
    /// Assumes that "to" can be reached from your position
    pub fn place(&self, to: Vec3) -> Option<TurtleCommand> {
        Some(match self.dig(to)? {
            TurtleCommand::Dig => TurtleCommand::Place,
            TurtleCommand::DigDown => TurtleCommand::PlaceDown,
            TurtleCommand::DigUp => TurtleCommand::PlaceUp,
            _ => None?
        })
    }

    /// Command to dig 
    /// Assumes that "to" can be dug from your position
    pub fn dig(&self, to: Vec3) -> Option<TurtleCommand> {

        // manhattan distance of 1 required to dig
        if (self.pos-to).abs().sum()!=1 {
            return None;
        }

        // not covered: pointing away from to

        Some(match self.pos.y - to.y {
            0 => TurtleCommand::Dig,
            1 => TurtleCommand::DigDown,
            -1 => TurtleCommand::DigUp,
            _ => None?
        })
    }

    pub fn manhattan(self, other: Self) -> i32 {
        self.pos.sub(other.pos).abs().sum()
    }
}

#[derive(Serialize, Deserialize, Clone, Hash, PartialEq, Eq, Copy, Debug)]
pub enum Direction {
    North,
    South,
    East,
    West,
}

impl Direction {
    pub fn left(self) -> Self {
        match self {
            Direction::North => Direction::West,
            Direction::South => Direction::East,
            Direction::East => Direction::North,
            Direction::West => Direction::South,
        }
    }

    pub fn right(self) -> Self {
        match self {
            Direction::North => Direction::East,
            Direction::South => Direction::West,
            Direction::East => Direction::South,
            Direction::West => Direction::North,
        }
    }
    pub fn unit(self) -> Vec3 {
        match self {
            Direction::North => Vec3::new(0, 0, -1),
            Direction::South => Vec3::new(0, 0, 1),
            Direction::East => Vec3::new(1, 0, 0),
            Direction::West => Vec3::new(-1, 0, 0),
        }
    }
}

/// closest valid state to the given point from where you are
pub fn nearest(from: Vec3, to: Vec3) -> Position {
    let diff = to.xz()-from.xz();
    
    let dir = if diff.x.abs() > diff.y.abs() {
        if diff.x > 0 {
            Direction::East
        } else {
            Direction::West
        }
    } else {
        if diff.y > 0 {
            Direction::South
        } else {
            Direction::South
        }
    };
    Position {
        pos: to - dir.unit(),
        dir
    }
}
