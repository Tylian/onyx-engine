use std::collections::HashMap;

use common::network::{MapLayer, RemoteMap, RemoteTile, TileAttribute as RemoteAttribute};
use macroquad::prelude::*;
use thiserror::Error;

use crate::assets::Assets;
use crate::ensure;

const OFFSETS: &[(i32, i32)] = &[
    (0, -1), (1, 0), (0, 1), (-1, 0),
    (1, -1), (1, 1), (-1, 1), (-1, -1)
];

fn autotile_a(neighbors: u8) -> IVec2 {
    if neighbors == 0 { return ivec2(0, 0); };

    let neighbors = neighbors & (1 | 8 | 128);
    let (x, y) = match neighbors {
        0 => (0, 2),
        128 => (0, 2),
        1 => (0, 4),
        8 => (2, 2),
        9 => (2, 0),
        137 => (2, 4),
        136 => (2, 2),
        129 => (0, 4),
        _ => unreachable!("autotile_a: {neighbors}")
    };
    ivec2(x, y)
}

fn autotile_b(neighbors: u8) -> IVec2 {
    if neighbors == 0 { return ivec2(1, 0); };

    let neighbors = neighbors & (1 | 2 | 16);
    let (x, y) = match neighbors {
        0 => (3, 2),
        16 => (3, 2),
        1 => (3, 4),
        2 => (1, 2),
        3 => (3, 0),
        19 => (1, 4),
        18 => (1, 2),
        17 => (3, 4),
        _ => unreachable!("autotile_b: {neighbors}")
    };
    ivec2(x, y)
}

fn autotile_c(neighbors: u8) -> IVec2 {
    if neighbors == 0 { return ivec2(0, 1); };

    let neighbors = neighbors & (4 | 8 | 64);
    let (x, y) = match neighbors {
        0 => (0, 5),
        64 => (0, 5),
        4 => (0, 3),
        8 => (2, 5),
        12 => (2, 1),
        76 => (2, 3),
        72 => (2, 5),
        68 => (0, 3),
        _ => unreachable!("autotile_c: {neighbors}")
    };
    ivec2(x, y)
}

fn autotile_d(neighbors: u8) -> IVec2 {
    if neighbors == 0 { return ivec2(1, 1); };
    let neighbors = neighbors & (4 | 2 | 32);
    let (x, y) = match neighbors {
        0 => (3, 5),
        32 => (3, 5),
        4 => (3, 3),
        2 => (1, 5),
        6 => (3, 1),
        38 => (1, 3),
        34 => (1, 5),
        36 => (3, 3),
        _ => unreachable!("autotile_d: {neighbors}")
    };
    ivec2(x, y)
}

#[derive(Copy, Clone)]
pub enum Tile {
    Empty,
    Basic(IVec2),
    Autotile {
        base: IVec2,
        cache: [IVec2; 4],
    }
}

impl Default for Tile {
    fn default() -> Self {
        Tile::Empty
    }
}

impl Tile {
    pub fn empty() -> Self {
        Self::Empty
    }
    pub fn basic(uv: IVec2) -> Self {
        Self::Basic(uv)
    }
    pub fn autotile(uv: IVec2) -> Self {
        Self::Autotile {
            base: uv,
            cache: Default::default()
        }
    }

    fn get_uv(&self) -> Option<IVec2> {
        match *self {
            Tile::Empty => None,
            Tile::Basic(uv) => Some(uv),
            Tile::Autotile { base, .. } => Some(base),
        }
    }

    pub fn update_autotile(&mut self, neighbors: u8) {
        if let Self::Autotile { base, cache } = self {
            let base = *base * 2;
            *cache = [
                base + autotile_a(neighbors),
                base + autotile_b(neighbors),
                base + autotile_c(neighbors),
                base + autotile_d(neighbors),
            ];
        } 
    }

    pub fn draw(&self, position: Vec2, assets: &Assets) {
        const A: (f32, f32) = (0.0, 0.0);
        const B: (f32, f32) = (24.0, 0.0);
        const C: (f32, f32) = (0.0, 24.0);
        const D: (f32, f32) = (24.0, 24.0);

        match self {
            Tile::Empty => (),
            Tile::Basic(uv) => self.draw_tile(position, *uv, assets),
            Tile::Autotile { cache, .. } => {
                self.draw_subtile(position + A.into(), cache[0], assets);
                self.draw_subtile(position + B.into(), cache[1], assets);
                self.draw_subtile(position + C.into(), cache[2], assets);
                self.draw_subtile(position + D.into(), cache[3], assets);
            },
        }
    }

    fn draw_tile(&self, position: Vec2, uv: IVec2, assets: &Assets) {
        let uv = uv * 48;
        let source = Rect::new(uv.x as f32, uv.y as f32, 48.0, 48.0);

        draw_texture_ex(
            assets.tileset,
            position.x,
            position.y,
            WHITE,
            DrawTextureParams {
                source: Some(source),
                ..Default::default()
            }
        );
    }

    fn draw_subtile(&self, position: Vec2, uv: IVec2, assets: &Assets) {
        let uv = uv * 24;
        let source = Rect::new(uv.x as f32, uv.y as f32, 24.0, 24.0);

        draw_texture_ex(
            assets.tileset,
            position.x,
            position.y,
            WHITE,
            DrawTextureParams {
                source: Some(source),
                ..Default::default()
            }
        );
    }
}

#[derive(Copy, Clone)]
pub enum TileAttribute {
    None,
    Blocked,
}

impl Default for TileAttribute {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Clone)]
pub struct Map {
    width: u32,
    height: u32,
    layers: HashMap<MapLayer, Vec<Tile>>,
    attributes: Vec<TileAttribute>
}

impl Map {
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height).try_into().unwrap();
        let mut layers = HashMap::new();

        for layer in MapLayer::iter() {
            layers.insert(layer, vec![Default::default(); size]);
        }

        Self {
            width,
            height,
            layers,
            attributes: vec![Default::default(); size],
        }
    }

    pub fn valid(&self, pos: IVec2) -> bool {
        pos.x >= 0 && pos.x < self.width as i32 && pos.y >= 0 && pos.y < self.height as i32
    }

    fn index(&self, position: IVec2) -> Option<usize> {
        if !self.valid(position) {
            return None;
        }

        Some((position.x as u32 + position.y as u32 * self.width) as usize)
    }

    pub fn tile(&self, layer: MapLayer, position: IVec2) -> Option<&Tile> {
        self.index(position).map(|index| self.layers[&layer].get(index).unwrap())
    }

    pub fn tile_mut(&mut self, layer: MapLayer, position: IVec2) -> Option<&mut Tile> {
        self.index(position).map(|index| self.layers.get_mut(&layer).unwrap().get_mut(index).unwrap())
    }

    pub fn set_tile(&mut self, layer: MapLayer, position: IVec2, tile: Tile) {
        self.index(position).map(|index| {
            self.layers.get_mut(&layer).unwrap()[index] = tile;
            //*self.layers.get_mut(&layer).unwrap().get_mut(index).unwrap() = tile;
        });
    }

    pub fn tiles(&self, layer: MapLayer) -> &[Tile] {
        self.layers.get(&layer).unwrap()
    }

    pub fn draw(&self, layer: MapLayer, assets: &Assets) {
        for (x, y) in itertools::iproduct!(0..self.width, 0..self.height) {
            let position = ivec2(x as i32, y as i32);
            let screen_position = position.as_f32() * 48.0;
            self.tile(layer, position).map(|tile| tile.draw(screen_position, assets));
        }
    }

    pub fn update_autotiles(&mut self) {
        // collecting all the data i need because we can't borrow self in the loop lmao
        let ground_map: Vec<_> = self.tiles(MapLayer::Ground).iter().map(Tile::get_uv).collect();
        let mask_map: Vec<_> = self.tiles(MapLayer::Mask).iter().map(Tile::get_uv).collect();
        let fringe_map: Vec<_> = self.tiles(MapLayer::Fringe).iter().map(Tile::get_uv).collect();

        let width = self.width;
        let height = self.height;

        let is_valid = |position: IVec2| position.x >= 0 && position.y >= 0 && position.x < width as i32 && position.y < height as i32;

        for (x, y) in itertools::iproduct!(0..self.width, 0..self.height) {
            let position = ivec2(x as i32, y as i32);
            for layer in MapLayer::iter() {
                let neighbor_map = match layer {
                    MapLayer::Ground => &ground_map,
                    MapLayer::Mask => &mask_map,
                    MapLayer::Fringe => &fringe_map,
                };

                if let Some(tile) = self.tile_mut(layer, position) {
                    let uv = tile.get_uv();
                    let mut neighbors = 0;
                    for (i, offset) in OFFSETS.iter().enumerate() {
                        let neighbor = position + IVec2::from(*offset);
                        if is_valid(neighbor) {
                            let neighbor = neighbor.as_u32();
                            let idx = (neighbor.x + neighbor.y * width) as usize;

                            match (uv, neighbor_map[idx]) {
                                (Some(a), Some(b)) if a == b => {
                                    neighbors |= 1 << i;
                                },
                                _ => ()
                            }
                        } else { // Auto-tiles look better when out of map tiles are assumed to be the same
                            neighbors |= 1 << i;
                        }
                    }
                    tile.update_autotile(neighbors);
                }
            }
            
        }
    }
}

#[derive(Debug, Error)]
pub enum MapError {
    #[error("size is incorrect")]
    IncorrectSize,
    #[error("the number of layers is incorrect")]
    IncorrectLayers,
}

impl TryFrom<RemoteMap> for Map {
    type Error = MapError;

    fn try_from(value: RemoteMap) -> Result<Self, Self::Error> {
        let size = (value.width * value.height) as usize;
        ensure!(value.attributes.len() == size, MapError::IncorrectSize);
        ensure!(value.layers.len() == MapLayer::count(), MapError::IncorrectLayers);

        let mut layers = HashMap::new();
        for (layer, contents) in value.layers {
            ensure!(contents.len() == size, MapError::IncorrectSize);
            layers.insert(layer, contents.into_iter().map(|t| t.into()).collect());
        }

        let mut map = Self {
            width: value.width,
            height: value.height,
            layers, 
            attributes: value.attributes.into_iter().map(|t| t.into()).collect(),
        };

        map.update_autotiles();

        Ok(map)
    }
}

impl From<RemoteTile> for Tile {
    fn from(remote: RemoteTile) -> Self {
        match remote {
            RemoteTile::Empty => Tile::Empty,
            RemoteTile::Basic(uv) => Tile::Basic(uv.into()),
            RemoteTile::Autotile(uv) => Tile::Autotile { base: uv.into(), cache: Default::default() },
        }
    }
}

impl From<RemoteAttribute> for TileAttribute {
    fn from(attribute: RemoteAttribute) -> Self {
        match attribute {
            RemoteAttribute::None => TileAttribute::None,
            RemoteAttribute::Blocked => TileAttribute::None,
        }
    }
}

// Note: It is considered an unrecoverable error to have a map that has an invalid size
impl From<Map> for RemoteMap {
    fn from(value: Map) -> Self {
        let size = (value.width * value.height) as usize;
        assert_eq!(value.attributes.len(), size);
        assert_eq!(value.layers.len(), MapLayer::count());

        let mut layers = HashMap::new();
        for (layer, contents) in value.layers {
            assert_eq!(contents.len(), size);
            layers.insert(layer, contents.into_iter().map(|t| t.into()).collect());
        }

        Self {
            width: value.width,
            height: value.height,
            layers, 
            attributes: value.attributes.into_iter().map(|t| t.into()).collect(),
        }
    }
}

impl From<Tile> for RemoteTile {
    fn from(tile: Tile) -> Self {
        match tile {
            Tile::Empty => RemoteTile::Empty,
            Tile::Basic(uv) => RemoteTile::Basic(uv.into()),
            Tile::Autotile { base, .. } => RemoteTile::Autotile(base.into()),
        }
    }
}

impl From<TileAttribute> for RemoteAttribute {
    fn from(attribute: TileAttribute) -> Self {
        match attribute {
            TileAttribute::None => RemoteAttribute::None,
            TileAttribute::Blocked => RemoteAttribute::None,
        }
    }
}