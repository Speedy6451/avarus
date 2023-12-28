use crate::blocks::Block;
use crate::blocks::Direction;
use crate::blocks::Position;
use crate::blocks::Vec3;
use crate::blocks::SharedWorld;
use crate::depot::DepotGuard;
use crate::depot::Depots;
use crate::paths::route_facing;
use crate::tasks::Scheduler;

use anyhow::Ok;

use anyhow;
use anyhow::Context;
use tokio::sync::OwnedMutexGuard;
use tracing::error;
use tracing::trace;
use tracing::warn;
use tracing::info;
use tokio::sync::Mutex;
use tokio::sync::OnceCell;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use tokio::time::timeout;

use super::LiveState;

use core::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;


use super::names::Name;

use serde::Deserialize;
use serde::Serialize;

use super::paths::route;

/// Time (ms) to wait for a command before letting a turtle go idle
const COMMAND_TIMEOUT:  u64 = 0o372;
/// Time (s) between turtle polls when idle
pub const IDLE_TIME: u32 = 3;
/// Times to attempt a route before giving up 
pub const RETRIES: usize = 42;

#[derive(Serialize, Deserialize)]
pub(crate) struct Turtle {
    pub(crate) name: Name,
    pub(crate) fuel: usize,
    pub(crate) fuel_limit: usize,
    /// movement vector of last given command
    pub(crate) queued_movement: Vec3,
    pub(crate) position: Position,
    pub(crate) pending_update: bool,
    #[serde(skip)]
    callback: Option<oneshot::Sender<TurtleInfo>>,
    #[serde(skip)]
    sender: Option<Arc<Sender>>,
    #[serde(skip)]
    receiver: Option<Receiver>,
}

#[derive(Debug)]
pub struct TurtleInfo {
    pub name: Name,
    pub pos: Position,
    pub fuel: usize,
    /// Block name
    pub ahead: String,
    pub above: String,
    pub below: String,
    pub ret: TurtleCommandResponse,
}

impl TurtleInfo {
    fn from_update(update: TurtleUpdate, name: Name, pos: Position) -> Self {
        Self { name, pos,
        fuel: update.fuel, ahead: update.ahead, above: update.above, below: update.below, ret: update.ret }
    }
}

pub type Sender = mpsc::Sender<(TurtleCommand, oneshot::Sender<TurtleInfo>)>;
pub type Receiver = mpsc::Receiver<(TurtleCommand, oneshot::Sender<TurtleInfo>)>;

impl Default for Turtle {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel(1);
        Self { 
            name: Name::from_num(0),
            fuel: Default::default(),
            fuel_limit: Default::default(),
            queued_movement: Default::default(),
            position: Position::new(Vec3::zeros(), Direction::North),
            pending_update: Default::default(),
            callback: None,
            sender: Some(Arc::new(sender)),
            receiver: Some(receiver),
        }
    }
}

impl Turtle {
    pub(crate) fn new(id: u32, position: Position, fuel: usize, fuel_limit: usize) -> Self {
        Self {
            name: Name::from_num(id),
            fuel,
            fuel_limit,
            queued_movement: Vec3::new(0, 0, 0),
            position,
            pending_update: true,
            ..Default::default()

        }
    }

    /// Similar turtle for serialization
    pub fn info(&self) -> Self {
        Self {
            name: self.name,
            fuel: self.fuel,
            fuel_limit: self.fuel_limit,
            position: self.position,
            pending_update: self.pending_update,
            queued_movement: self.queued_movement,
            ..Default::default()
        }
    }

    pub fn with_channel(id: u32, position: Position, fuel: usize, fuel_limit: usize, sender: Sender, receiver: Receiver) -> Self {
        Self {
            name: Name::from_num(id),
            fuel,
            fuel_limit,
            queued_movement: Vec3::new(0, 0, 0),
            position,
            pending_update: true,
            sender: Some(Arc::new(sender)),
            receiver: Some(receiver),
            ..Default::default()

        }
    }
}

#[derive(Clone)]
pub struct TurtleCommander {
    sender: Arc<Sender>,
    world: SharedWorld,
    depots: Depots,
    // everything below is best-effort
    // TODO: make not bad
    pos: Arc<RwLock<Position>>,
    fuel: Arc<AtomicUsize>,
    max_fuel: Arc<AtomicUsize>,
    name: Arc<OnceCell<Name>>,
    inventory: Arc<RwLock<Option<Vec<Option<InventorySlot>>>>>,
    tasks: Arc<Mutex<Scheduler>>, // this feels subpar, I feel like a mpsc would do better
}

impl fmt::Debug for TurtleCommander {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Turtle {} ", self.name().to_str())?;
        write!(f, "fuel: {} ", self.fuel())?;
        write!(f, "fuel_limit: {} ", self.fuel_limit())
    }
}

impl TurtleCommander {
    pub async fn new(turtle: Name, state: &LiveState) -> Option<TurtleCommander> {
        let turtle = state.turtles.get(turtle.to_num() as usize)?.clone();
        let turtle = turtle.read().await;
        Some(TurtleCommander { 
            sender: turtle.sender.as_ref().unwrap().clone(),
            world: state.world.clone(),
            pos: Arc::new(RwLock::new(turtle.position)),
            fuel: Arc::new(AtomicUsize::new(turtle.fuel)),
            max_fuel: Arc::new(AtomicUsize::new(turtle.fuel_limit)),
            name: Arc::new(OnceCell::new_with(Some(turtle.name))),
            depots: state.depots.clone(),
            inventory: Default::default(),
            tasks: state.tasks.clone(),
        })
    }

    pub fn with_turtle(turtle: &Turtle, state: &LiveState) -> TurtleCommander {
        TurtleCommander { 
            sender: turtle.sender.as_ref().unwrap().clone(),
            world: state.world.clone(),
            pos: Arc::new(RwLock::new(turtle.position)),
            fuel: Arc::new(AtomicUsize::new(turtle.fuel)),
            max_fuel: Arc::new(AtomicUsize::new(turtle.fuel_limit)),
            name: Arc::new(OnceCell::new_with(Some(turtle.name))),
            depots: state.depots.clone(),
            inventory: Default::default(),
            tasks: state.tasks.clone(),
        }
    }

    #[tracing::instrument]
    pub async fn execute(&self, command: TurtleCommand) -> TurtleInfo {
        let (send, recv) = oneshot::channel::<TurtleInfo>();

        if let Err(_) = self.sender.to_owned().send((command.clone(),send)).await {
            error!("server disappeared"); // It's fine to continue, nobody 
                                          // is left to read garbage
        };

        let resp = recv.await.unwrap_or_else(|_| {
            error!("server disappearing"); 
            TurtleInfo::from_update(TurtleUpdate { fuel: self.fuel(), ahead: "".into(), above: "".into(), below: "".into(), ret: TurtleCommandResponse::Failure }, self.name(), Position::new(Vec3::zeros(), Direction::North))
        });

        // invalidate inventory when we run commands that modify it
        // this is not safe if you make a second TurtleCommander
        if let TurtleCommandResponse::Success = resp.ret {
            if match command {
                TurtleCommand::Wait(_) => false,
                TurtleCommand::Forward(_) => false,
                TurtleCommand::Backward(_) => false,
                TurtleCommand::Up(_) => false,
                TurtleCommand::Down(_) => false,
                TurtleCommand::Left => false,
                TurtleCommand::Right => false,
                _ => true,
            } {
                *self.inventory.write().await = None;
            }
        }

        let mut pos = self.pos.write().await;
        *pos = resp.pos;
        self.fuel.store(resp.fuel, std::sync::atomic::Ordering::SeqCst);
        resp
    }

    pub fn name(&self) -> Name {
        self.name.get().unwrap().clone()
    }

    pub async fn pos(&self) -> Position {
        self.pos.read().await.clone()
    }

    pub fn fuel(&self) -> usize {
        self.fuel.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn fuel_limit(&self) -> usize {
        self.max_fuel.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn world(&self) -> SharedWorld {
        self.world.clone()
    }

    pub async fn scheduler(&self) -> OwnedMutexGuard<Scheduler> {
        self.tasks.clone().lock_owned().await
    }

    pub async fn inventory(&self) -> Vec<Option<InventorySlot>> {
        let mut inventory = self.inventory.write().await;

        if inventory.is_some() {
            return inventory.clone().unwrap();
        }

        let mut scan = Vec::new();

        for i in 1..=16 {
            match self.execute(TurtleCommand::ItemInfo(i)).await.ret {
                TurtleCommandResponse::Item(item) => {
                    scan.push(Some(item));
                }
                TurtleCommandResponse::None => {
                    scan.push(None);
                }
                _ => {
                    error!("inventory scan for #{} is going sideways", self.name().to_str());
                    scan.push(None);
                },
            }
        };

        *inventory = Some(scan.clone());
        scan
    }

    #[tracing::instrument(skip(self))]
    pub async fn dock(&self) -> usize {
        let res = Depots::dock(&self.depots, self.to_owned()).await;
        if let Some(fuel) = res {
            return fuel;
        };
        error!("dock failed");
        self.fuel()
    }

    pub async fn try_dock(&self) -> Option<usize> {
        self.depots.dock(self.clone()).await 
    }

    pub async fn get_depot(&self) -> DepotGuard {
        self.depots.nearest(self.pos().await).await
    }

    #[tracing::instrument(skip(self))]
    pub async fn goto(&self, pos: Position) -> Option<()> {
        let mut recent = self.pos().await;
        let world = self.world.clone();
        let mut attempts = RETRIES + 1;
        loop {
            if recent == pos {
                break;
            }

            attempts -= 1;
            if attempts == 0 {
                error!("goto {pos:?} failed");
                break;
            }

            let routing = route(recent, pos, &world);
            let route = routing.await?;

            trace!("using route: {route:#?}");

            let steps: Vec<TurtleCommand> = route.iter().map_windows(|[from,to]| from.difference(**to).unwrap()).collect();

            'route: for (next_position, command) in route.into_iter().skip(1).zip(steps) {
                // reroute if the goal point is not empty before moving
                // valid routes will explicitly tell you to break ground

                if world.occupied(next_position.pos).await {
                    if world.garbage(next_position.pos).await {
                        match recent.dig(next_position.pos) {
                            Some(command) => self.execute(command).await,
                            None => break 'route,
                        };
                    } else {
                        break 'route;
                    }
                }

                let state = self.execute(command.clone()).await;

                if let TurtleCommandResponse::Failure =  state.ret {
                    if let TurtleCommand::Backward(_) = command {
                        // turn around if you bump your rear on something
                        self.execute(TurtleCommand::Left).await;
                        recent = self.execute(TurtleCommand::Left).await.pos;
                    }
                    break 'route;
                }

                recent = state.pos;
            }
        }
        Some(())
    }

    #[tracing::instrument]
    pub async fn goto_adjacent(&self, pos: Vec3) -> Option<Position> {
        let mut recent = self.pos().await;
        let world = self.world.clone();
        let mut attempts = RETRIES +1;
        loop {
            
            if pos == recent.dir.unit() + recent.pos 
                || pos == recent.pos + Vec3::y()
                || pos == recent.pos - Vec3::y()
            {
                break;
            }

            attempts -= 1;
            if attempts == 0 {
                error!("adjacent {pos:?} failed");
                break;
            }

            let routing = route_facing(recent, pos, &world);
            let route = routing.await?;

            let steps: Vec<TurtleCommand> = route.iter().map_windows(|[from,to]| from.difference(**to).unwrap()).collect();

            'route: for (next_position, command) in route.into_iter().skip(1).zip(steps) {
                if recent.pos != next_position.pos && world.occupied(next_position.pos).await {
                    if world.garbage(next_position.pos).await {
                        let command = recent.dig(next_position.pos);
                        match command {
                            Some(command) => self.execute(command).await,
                            None => {
                                warn!("next location not diggable");
                                break 'route
                            },
                        };
                    } else {
                        warn!("non destructible block on route: {} at {:?}", world.get(next_position.pos).await.unwrap().name, next_position);
                        break 'route;
                    }
                }

                let state = self.execute(command.clone()).await;

                if let TurtleCommandResponse::Failure =  state.ret {
                    if let TurtleCommand::Backward(_) = command {
                        // turn around if you bump your rear on something
                        self.execute(TurtleCommand::Left).await;
                        recent = self.execute(TurtleCommand::Left).await.pos;
                    }
                    warn!("command failed");
                    break 'route;
                }

                recent = state.pos;
            }
        }
        Some(recent)
    }
}

pub(crate) async fn process_turtle_update(
    id: u32,
    state: &LiveState,
    update: TurtleUpdate,
) -> Option<TurtleCommand> {
    let mut turtle =  match state
        .turtles
        .get(id as usize) {
            Some(turtle) => turtle.write().await,
            None => {
                error!("nonexisting turtle {id}");
                return Some(TurtleCommand::Poweroff);
            },
    };
    
    let world = &state.world;

    if turtle.pending_update {
        turtle.pending_update = false;
        return Some(TurtleCommand::Update);
    }

    if turtle.fuel > update.fuel {
        let diff = turtle.fuel - update.fuel;

        let delta = turtle.queued_movement * diff as i32;

        turtle.position.pos += delta;
        turtle.queued_movement = Vec3::zeros();
    }
    turtle.fuel = update.fuel;

    let above = Block {
        name: update.above.clone(),
        pos: turtle.position.pos + Vec3::y(),
    };
    world.set(above.clone()).await;

    let ahead = Block {
        name: update.ahead.clone(),
        pos: turtle.position.pos + turtle.position.dir.unit(),
    };
    world.set(ahead.clone()).await;

    let below = Block {
        name: update.below.clone(),
        pos: turtle.position.pos - Vec3::y(),
    };
    world.set(below.clone()).await;

    let info = TurtleInfo::from_update(update, turtle.name.clone(), turtle.position.clone());

    if let TurtleCommandResponse::Failure = info.ret {
        info!("{}: command failure", turtle.name.to_str());
    }

    if let Some(send) = turtle.callback.take() {
        send.send(info).unwrap_or_else(|_| warn!("task cancelled"));
    }

    if let Some(recv) = turtle.receiver.as_mut() {
        let next = timeout(Duration::from_millis(COMMAND_TIMEOUT), recv.recv());
        if let Some((cmd, ret)) = next.await.ok().flatten() {
            turtle.callback = Some(ret);

            match cmd {
                TurtleCommand::Left => turtle.position.dir = turtle.position.dir.left(),
                TurtleCommand::Right => turtle.position.dir = turtle.position.dir.right(),
                _ => {}
            }
            turtle.queued_movement = cmd.unit(turtle.position.dir);
            info!("{}: {cmd:?}", turtle.name.to_str());
            return Some(cmd);
        }
    }

    trace!("{} idle, connected", turtle.name.to_str());
    None
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TurtleCommand {
    Wait(u32),
    Forward(u32),
    Backward(u32),
    Up(u32),
    Down(u32),
    Left,
    Right,
    Dig,
    DigUp,
    DigDown,
    PlaceUp,
    Place,
    PlaceDown,
    /// Count
    DropFront(u32),
    DropUp(u32),
    DropDown(u32),
    SuckFront(u32),
    SuckUp(u32),
    SuckDown(u32),
    Select(u32),
    /// Slot in inventory
    ItemInfo(u32),
    Update,
    Poweroff,
    Refuel,
    CycleFront,
    /// Name of the computer in front of the one commanded
    NameFront,
    /// Name of the current computer
    Name,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct InventorySlot {
    pub(crate) name: String,
    pub(crate) count: u32,
}

// bodge
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TurtleString {
    pub string: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) enum TurtleCommandResponse {
    None,
    Success,
    Failure,
    Item(InventorySlot),
    Inventory(Vec<InventorySlot>),
    Name(TurtleString),
}

impl TurtleCommand {
    pub(crate) fn delta(&self, direction: Direction) -> Vec3 {
        let dir = direction.unit();
        match self {
            TurtleCommand::Forward(count) => dir * *count as i32,
            TurtleCommand::Backward(count) => -dir * *count as i32,
            TurtleCommand::Up(count) => Vec3::y() * *count as i32,
            TurtleCommand::Down(count) => -Vec3::y() * *count as i32,
            _ => Vec3::zeros(),
        }
    }

    pub(crate) fn unit(&self, direction: Direction) -> Vec3 {
        let dir = direction.unit();
        match self {
            TurtleCommand::Forward(_) => dir,
            TurtleCommand::Backward(_) => -dir,
            TurtleCommand::Up(_) => Vec3::y(),
            TurtleCommand::Down(_) => -Vec3::y(),
            _ => Vec3::zeros(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct TurtleUpdate {
    pub(crate) fuel: usize,
    /// Block name
    pub(crate) ahead: String,
    pub(crate) above: String,
    pub(crate) below: String,
    pub(crate) ret: TurtleCommandResponse,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TurtleRegister {
    pub(crate) fuel: usize,
    pub(crate) fuellimit: usize,
    pub(crate) position: Vec3,
    pub(crate) facing: Direction,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TurtleResponse {
    pub(crate) name: String,
    pub(crate) id: u32,
    pub(crate) command: TurtleCommand,
}
