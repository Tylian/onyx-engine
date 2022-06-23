use std::{
    collections::{HashMap, VecDeque},
    ffi::OsStr,
    fs,
    sync::RwLock,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use euclid::default::{Point2D, Rect, Size2D, Vector2D};
use onyx_common::{
    network::{
        AreaData, ChatMessage, ClientId, ClientMessage, Direction, Map as NetworkMap, MapId,
        PlayerData as NetworkPlayerData, ServerMessage,
    },
    SPRITE_SIZE, TILE_SIZE,
};

use crate::networking::{Message, NetworkSignal, Networking};

mod networking;

#[derive(Clone)]
struct PlayerData {
    name: String,
    sprite: u32,
    position: Point2D<f32>,
    direction: Direction,
    velocity: Option<Vector2D<f32>>,
    map: MapId,
    last_message: Instant,
}

impl From<PlayerData> for NetworkPlayerData {
    fn from(other: PlayerData) -> Self {
        Self {
            name: other.name,
            sprite: other.sprite,
            position: other.position.into(),
            direction: other.direction,
        }
    }
}

#[derive(Default)]
struct WarpParams {
    initial: bool,
    position: Option<Point2D<f32>>,
    direction: Option<Direction>,
    velocity: Option<Option<Vector2D<f32>>>,
}

struct GameServer {
    network: RwLock<Networking>,
    network_queue: VecDeque<Message>,
    players: HashMap<ClientId, PlayerData>,
    maps: HashMap<MapId, NetworkMap>,
    time: Instant,
    /// Time since last update
    dt: Duration,
}

impl GameServer {
    pub fn new() -> Result<Self> {
        let mut network = Networking::new();
        network.listen("0.0.0.0:3042");

        let maps = Self::load_maps()?;

        Ok(Self {
            network: RwLock::new(network),
            network_queue: VecDeque::new(),
            players: HashMap::new(),
            time: Instant::now(),
            dt: Duration::ZERO,
            maps,
        })
    }

    pub fn run(self) {
        self.game_loop();
    }

    pub fn load_maps() -> Result<HashMap<MapId, NetworkMap>> {
        let mut maps = HashMap::new();
        for entry in fs::read_dir("./data/maps")? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                let bytes = fs::read(&path)?;
                let map = bincode::deserialize::<NetworkMap>(&bytes)?;

                #[cfg(debug_assertions)]
                if path.file_name().and_then(OsStr::to_str) != Some(&format!("{}.bin", map.id.0)) {
                    log::warn!(
                        "Map loaded but the name didn't match it's id: {:?} {}",
                        map.id,
                        path.display()
                    );
                }

                maps.insert(map.id, map);
            }
        }

        // ensure there's a "start" map
        maps.entry(MapId::start())
            .or_insert_with(|| NetworkMap::new(MapId::start(), 20, 15));
        Ok(maps)
    }

    pub fn save_map(&self, id: MapId) -> anyhow::Result<()> {
        let map = self.maps.get(&id).ok_or_else(|| anyhow!("map doesn't exist"))?;
        let bytes = bincode::serialize(&map)?;
        log::debug!("saving map {}: {} bytes", id.0, bytes.len());
        fs::write(format!("./data/maps/{}.bin", id.0), bytes)?;
        Ok(())
    }

    pub fn load_player(&self, id: &str) -> PlayerData {
        PlayerData {
            name: String::new(),
            sprite: 0,
            position: Point2D::new(10. * 48., 7. * 48.),
            direction: Direction::South,
            map: MapId::start(),
            velocity: None,
            last_message: self.time,
        }
    }

    fn handle_disconnect(&mut self, client_id: ClientId) {
        if let Some(player) = self.players.remove(&client_id) {
            self.queue(Message::list(
                self.players
                    .iter()
                    .filter(|(_, data)| data.map == player.map)
                    .map(|(&cid, _)| cid)
                    .collect::<Vec<_>>(),
                ServerMessage::PlayerLeft(client_id),
            ));

            let goodbye = ServerMessage::Message(ChatMessage::Server(format!("{} has left the game.", &player.name)));
            self.queue(Message::exclude(client_id, goodbye));
        }
    }

    fn handle_message(&mut self, client_id: ClientId, message: ClientMessage) {
        log::debug!("{:?}: {:?}", client_id, message);
        if !self.players.contains_key(&client_id) && !matches!(message, ClientMessage::Hello(_, _)) {
            log::error!("Client sent a packet when it's not connected");
            return;
        }

        match message {
            ClientMessage::Hello(name, sprite) => {
                let mut player = self.load_player(&name); // todo lol
                player.name = name;
                player.sprite = sprite;

                // Save their data
                self.players.insert(client_id, player.clone());

                // Send them their ID
                self.queue(Message::only(client_id, ServerMessage::Hello(client_id)));

                self.warp_player(
                    client_id,
                    player.map,
                    WarpParams {
                        initial: true,
                        ..Default::default()
                    },
                );

                // Send welcome message
                self.queue(Message::only(
                    client_id,
                    ServerMessage::Message(ChatMessage::Server("Welcome to Game™!".to_owned())),
                ));

                // Send join message
                self.queue(Message::exclude(
                    client_id,
                    ServerMessage::Message(ChatMessage::Server(format!("{} has joined the game.", &player.name))),
                ));
            }

            ClientMessage::Message(text) => {
                if let Some(player) = self.players.get(&client_id) {
                    let full_text = format!("{}: {}", player.name, text);
                    let packet = ServerMessage::Message(ChatMessage::Say(full_text));
                    self.queue(Message::everybody(packet));
                }
            }
            ClientMessage::RequestMap => {
                if let Some(map_id) = self.players.get(&client_id).map(|p| p.map) {
                    let map = self
                        .maps
                        .entry(map_id)
                        .or_insert_with(|| NetworkMap::new(map_id, 20, 15));

                    let packet = ServerMessage::MapData(Box::new(map.clone()));
                    self.queue(Message::only(client_id, packet));
                }
            }
            ClientMessage::SaveMap(map) => {
                let map_id = self.players.get(&client_id).map(|p| p.map).unwrap();
                self.maps.insert(map_id, *map.clone());
                if let Err(e) = self.save_map(map_id) {
                    log::error!("Couldn't save map {e}");
                }

                let packet = Message::list(
                    self.players
                        .iter()
                        .filter(|(_, data)| data.map == map_id)
                        .map(|(&cid, _)| cid)
                        .collect::<Vec<_>>(),
                    ServerMessage::MapData(map),
                );
                self.queue(packet);
            }
            ClientMessage::Move {
                position,
                direction,
                velocity,
            } => {
                let player = self.players.get_mut(&client_id).unwrap();
                player.position = position.into();
                player.velocity = velocity.map(Into::into);

                let packet = ServerMessage::PlayerMove {
                    client_id,
                    position,
                    direction,
                    velocity,
                };

                let map_id = player.map;
                let players = self
                    .players
                    .iter()
                    .filter(|(cid, data)| data.map == map_id)
                    .map(|(&cid, _)| cid)
                    .collect();
                self.queue(Message::list(players, packet));
            }
            ClientMessage::Warp(map_id, position) => {
                self.warp_player(
                    client_id,
                    map_id,
                    WarpParams {
                        position: position.map(Into::into),
                        velocity: None,
                        ..Default::default()
                    },
                );
            }
            ClientMessage::MapEditor => {
                let map_id = self.players.get(&client_id).map(|p| &p.map).unwrap();
                let maps = self
                    .maps
                    .iter()
                    .map(|(&id, map)| (id, map.settings.name.clone()))
                    .collect::<HashMap<_, _>>();
                let map = self.maps.get(map_id).unwrap();

                let id = *map_id;
                let width = map.width;
                let height = map.height;
                let settings = map.settings.clone();

                self.queue(Message::only(
                    client_id,
                    ServerMessage::MapEditor {
                        maps,
                        id,
                        width,
                        height,
                        settings,
                    },
                ));
            }
        }
    }

    fn game_loop(mut self) {
        loop {
            let now = Instant::now();
            let dt = now - self.time;
            self.time = now;
            self.dt = dt;

            // networking
            while let Some(signal) = self.try_recv() {
                match signal {
                    NetworkSignal::Message(client_id, message) => self.handle_message(client_id, message),
                    NetworkSignal::Connected(_client_id) => (),
                    NetworkSignal::Disconnected(client_id) => self.handle_disconnect(client_id),
                }
            }

            // game loop
            self.update_players();

            // finalizing
            self.send_all();
            std::thread::sleep(Duration::from_secs_f64(1.0 / 60.0));
        }
    }

    fn update_players(&mut self) {
        let mut packets = Vec::new();
        let dt = self.dt;

        for (id, player) in &mut self.players {
            let map = match self.maps.get(&player.map) {
                Some(map) => map,
                None => continue,
            };

            if let Some(velocity) = player.velocity {
                let offset = velocity * dt.as_secs_f32();
                let new_position = player.position + offset;

                // only block on the bottom half of the sprite, feels better
                let sprite = Rect::new(
                    Point2D::new(new_position.x, new_position.y + SPRITE_SIZE as f32 / 2.0),
                    Size2D::new(SPRITE_SIZE as f32, SPRITE_SIZE as f32 / 2.0),
                )
                .to_box2d();

                let map_size = Size2D::new(
                    map.width as f32 * TILE_SIZE as f32,
                    map.height as f32 * TILE_SIZE as f32,
                ); // todo map method
                let map_box = Rect::new(Point2D::zero(), map_size).to_box2d();

                let valid = map_box.contains_box(&sprite)
                    && !map.areas.iter().any(|attrib| {
                        let box2d = Rect::new(attrib.position.into(), attrib.size.into()).to_box2d();
                        attrib.data == AreaData::Blocked && box2d.intersects(&sprite)
                    });

                if valid {
                    player.position = new_position;
                }
            }

            let sprite = Rect::new(
                Point2D::new(player.position.x, player.position.y + SPRITE_SIZE as f32 / 2.0),
                Size2D::new(SPRITE_SIZE as f32, SPRITE_SIZE as f32 / 2.0),
            )
            .to_box2d();

            for attrib in map.areas.iter() {
                match &attrib.data {
                    AreaData::Log(message) => {
                        let box2d = Rect::new(attrib.position.into(), attrib.size.into()).to_box2d();
                        if box2d.intersects(&sprite) && player.last_message.elapsed() > Duration::from_secs(1) {
                            let message = ChatMessage::Server(message.clone());
                            packets.push(Message::only(*id, ServerMessage::Message(message)));
                            player.last_message = self.time;
                        }
                    }
                    AreaData::Blocked => (),
                }
            }
        }
    }

    /// Warps the player to a specific map, sending all the correct packets
    fn warp_player(&mut self, client_id: ClientId, map_id: MapId, params: WarpParams) {
        if !self.players.contains_key(&client_id) {
            return;
        }
        if !params.initial {
            let list = self
                .players
                .iter()
                .filter(|(&cid, data)| cid != client_id && data.map == map_id)
                .map(|(&cid, _)| cid)
                .collect();

            self.queue(Message::list(list, ServerMessage::PlayerLeft(client_id)));
        }

        self.players.get_mut(&client_id).unwrap().map = map_id;
        let revision = self.maps.get(&map_id).map(|m| m.settings.revision).unwrap_or(0);

        self.queue(Message::only(client_id, ServerMessage::ChangeMap(map_id, revision)));

        let packets = self
            .players
            .iter()
            .filter(|(_, data)| data.map == map_id)
            .map(|(&cid, data)| ServerMessage::PlayerJoined(cid, data.clone().into()))
            .collect::<Vec<_>>();

        for packet in packets {
            self.queue(Message::only(client_id, packet));
        }

        self.queue(Message::list(
            self.players
                .iter()
                .filter(|(_, data)| data.map == map_id)
                .map(|(&cid, _)| cid)
                .collect::<Vec<_>>(),
            ServerMessage::PlayerJoined(client_id, self.players.get(&client_id).unwrap().clone().into()),
        ));

        let packet = self
            .players
            .get(&client_id)
            .map(|player| ServerMessage::PlayerMove {
                client_id,
                position: params.position.unwrap_or(player.position).into(),
                direction: params.direction.unwrap_or(player.direction),
                velocity: params.velocity.unwrap_or(player.velocity).map(Into::into),
            })
            .unwrap();

        self.queue(Message::list(
            self.players
                .iter()
                .filter(|(_, data)| data.map == map_id)
                .map(|(&cid, _)| cid)
                .collect::<Vec<_>>(),
            packet,
        ));
    }

    // Specifically created to avoid scope issues
    fn try_recv(&self) -> Option<NetworkSignal> {
        self.network.read().unwrap().try_recv()
    }

    pub fn queue(&mut self, message: Message) {
        self.network_queue.push_back(message);
    }

    pub fn send_all(&mut self) {
        let network = self.network.read().unwrap();
        for message in self.network_queue.drain(..) {
            message.write(&network);
        }
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(debug_assertions)]
    simple_logger::init_with_level(log::Level::Debug).unwrap();

    #[cfg(not(debug_assertions))]
    simple_logger::init_with_level(log::Level::Warn).unwrap();

    let game_server = GameServer::new()?;
    game_server.run();

    Ok(())
}
