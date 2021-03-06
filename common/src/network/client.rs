use mint::{Point2, Vector2};
use serde::{Deserialize, Serialize};

use super::{ChatChannel, Direction, Map};

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub enum Packet {
    CreateAccount {
        username: String,
        password: String,
        character_name: String,
    },
    Login {
        username: String,
        password: String,
    },
    Move {
        position: Point2<f32>,
        direction: Direction,
        velocity: Option<Vector2<f32>>,
    },
    ChatMessage(ChatChannel, String),
    RequestMap,
    SaveMap(Box<Map>),
    Warp(String, Option<Point2<f32>>),
    MapEditor(bool),
}
