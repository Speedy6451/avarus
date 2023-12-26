extern crate test;
use std::{sync::Arc, ops::Sub, collections::HashMap};

use anyhow::{Ok, anyhow};
use nalgebra::Vector3;
use rstar::{PointDistance, RTree, RTreeObject, AABB, Envelope};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, OwnedRwLockReadGuard};

use crate::{turtle::TurtleCommand, paths::{self, TRANSPARENT}};

const CHUNK_SIZE: usize = 4;
const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;
const CHUNK_VEC: Vec3  = Vec3::new(CHUNK_SIZE as i32, CHUNK_SIZE as i32, CHUNK_SIZE as i32);

#[derive(Serialize, Deserialize)]
pub struct World { // TODO: make r-trees faster than this, for my sanity
    index: HashMap<Vec3, usize>,
    data: Vec<Chunk>,
    last: Option<usize>,
}

impl World {
    pub fn new() -> Self {
        World{
           index:  HashMap::new(),
           data: Vec::new(),
           last: None,
        }
    }
    pub fn get(&self, block: Vec3) -> Option<Block> {
        let chunk = self.get_chunk(block)?;
        Some(chunk.get(block)?)
    }

    pub fn set(&mut self, block: Block) {
        let chunk_coords = block.pos.map(|n| i32::div_floor(n,CHUNK_SIZE as i32));

        let chunk = self.last
            .filter(|n| self.data[*n].contains(&block.pos))
            .or_else(|| {
                self.index.get(&chunk_coords).map(|c| *c)
            })
            .map(|n| *self.last.insert(n));

        match chunk {
            Some(chunk) => {
                self.data[chunk].set(block).unwrap();
            },
            None => {
                let mut new_chunk = Chunk::new(chunk_coords);
                new_chunk.set(block).unwrap();
                self.data.push(new_chunk);
                self.index.insert(chunk_coords, self.data.len() - 1);
            },
        }
    }

    fn get_chunk(&self, block: Vec3) -> Option<&Chunk> {
        let block = block.map(|n| i32::div_floor(n,CHUNK_SIZE as i32));
        if let Some(last) = self.last {
            if self.data[last].contains(&block) {
                return Some(&self.data[last])
            }
        }
        self.index.get(&block).map(|i| &self.data[*i])
    }
}

#[derive(Clone)]
pub struct SharedWorld {
    state: Arc<RwLock<World>>, // interior mutability to get around the 
                              // questionable architecture of this project
}

impl SharedWorld {
    pub fn new() -> Self { Self { state: Arc::new(RwLock::new(World::new())) } }
    pub fn from_world(tree: World) -> Self { Self { state: Arc::new(RwLock::new(tree)) } }

    pub async fn get(&self, block: Vec3) -> Option<Block> {
        Some(self.state.read().await.get(block)?.clone())
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
    data: [[[Option<String>;CHUNK_SIZE];CHUNK_SIZE];CHUNK_SIZE]
}

impl Chunk {
    fn new(pos: Vec3) -> Self {
        let data :[[[Option<String>;CHUNK_SIZE];CHUNK_SIZE];CHUNK_SIZE]= Default::default();
        Self {
            pos,
            data
        }
    }

    fn set(&mut self, pos: Block) -> anyhow::Result<()> {
        let chunk = self.pos.component_mul(&CHUNK_VEC);
        if !self.contains(&pos.pos) {
            return Err(anyhow!("out of bounds"));
        }
        let local: Vector3<usize> = (pos.pos - chunk).map(|n| n as usize);

        self.data[local.x][local.y][local.z] = Some(pos.name);

        Ok(())
    }

    fn get(&self, pos: Vec3) -> Option<Block> {
        let chunk = self.pos.component_mul(&CHUNK_VEC);
        let local = pos - chunk;
        if !self.contains(&pos) {
            return None;
        }
        let local = local.map(|n| n as usize);

        Some(Block {
            name: self.data[local.x][local.y][local.z].clone()?,
            pos,
        })
    }

    fn contains(&self, pos:&Vec3) -> bool {
        let chunk = self.pos.component_mul(&CHUNK_VEC);
        let local = pos - chunk;
        local >= Vec3::zeros() && local < CHUNK_VEC
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

#[cfg(test)]
mod tests {
    use test::Bencher;

    use crate::mine::fill;

    use super::*;

    fn single_point(point: Vec3) {
        let mut world = World::new();
        world.set(Block { name: "a".to_string(), pos: point});

        assert_eq!("a", world.get(point).unwrap().name);
    }

    fn many(point: Vec3, size: Vec3) {
        let mut world = World::new();
        for i in 0..size.product() {
            let block = fill(size, i) + point;
            world.set(Block { name: i.to_string(), pos: block});
        }

        for i in 0..size.product() {
            let block = fill(size, i) + point;
            assert_eq!(i.to_string(), world.get(block).unwrap().name)
        }
    }

    #[test]
    fn origin() {
        single_point(Vec3::zeros())
    }
    #[test]
    fn big() {
        single_point(Vec3::new(1212,100,1292))
    }
    #[test]
    fn small() {
        single_point(Vec3::new(-1212,100,-1292))
    }

    #[test]
    fn positive_many() {
        many(Vec3::new(1212,100,1292), Vec3::new(100, 100, 100))
    }

    #[test]
    fn negative_many() {
        many(Vec3::new(-1212,100,-1292), Vec3::new(100, 100, 100))
    }

    #[bench]
    fn positive_several(b: &mut Bencher) {
        b.iter(||many(Vec3::new(1212,100,1292), Vec3::new(100, 1, 30)));
    }

    #[bench]
    fn positive_many_bench(b: &mut Bencher) {
        b.iter(||many(Vec3::new(1212,100,1292), Vec3::new(50, 50, 50)));
    }
}
