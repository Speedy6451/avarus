use rstar::{self, RTree, RTreeObject, AABB, PointDistance};
use serde::{Deserialize, Serialize};
use pathfinding::prelude::astar;

use crate::Vec3;

pub type World = RTree<Block>;

#[derive(Serialize, Deserialize)]
pub struct Block {
    pub name: String,
    pub pos: super::Vec3,
}


impl RTreeObject for Block {
    type Envelope = AABB<[i32;3]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.pos.into())
    }
}

impl PointDistance for Block {
    fn distance_2(
            &self,
            point: &[i32;3],
        ) -> i32 {
        (self.pos - Vec3::from(*point)).abs().sum()
    }
    
}
