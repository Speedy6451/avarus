use nalgebra::Vector3;
use rstar::{self, PointDistance, RTree, RTreeObject, AABB};
use serde::{Deserialize, Serialize};

pub type World = RTree<Block>;

#[derive(Serialize, Deserialize, Clone)]
pub struct Block {
    pub name: String,
    pub pos: Vec3,
}

impl RTreeObject for Block {
    type Envelope = AABB<[i32; 3]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.pos.into())
    }
}

impl PointDistance for Block {
    fn distance_2(&self, point: &[i32; 3]) -> i32 {
        (self.pos - Vec3::from(*point)).abs().sum()
    }
}

pub type Vec3 = Vector3<i32>;
pub type Position = (Vec3, Direction);

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
    (
        to - dir.unit(),
        dir
    )
}
