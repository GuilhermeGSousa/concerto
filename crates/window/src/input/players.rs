use std::collections::BTreeMap;
use std::fmt;

use concerto_ecs::{
    component::Component,
    events::{event_reader::EventReader, event_writer::EventWriter, Event},
    resource::{Res, ResMut, Resource},
};

use crate::events::GamepadConnection;
use crate::input::{GamepadButton, GamepadId, Input, KeyCode, PhysicalKey};

/// One local player's place, from 0.
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct PlayerIndex(u8);

impl PlayerIndex {
    pub const fn new(index: u8) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u8 {
        self.0
    }
}

/// A device that can be given to a player.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PlayerDevice {
    Gamepad(GamepadId),
    KeyboardMouse,
}

/// The devices one player holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PlayerDevices {
    pub gamepad: Option<GamepadId>,
    pub keyboard_mouse: bool,
}

impl PlayerDevices {
    fn holds(self, device: PlayerDevice) -> bool {
        match device {
            PlayerDevice::Gamepad(id) => self.gamepad == Some(id),
            PlayerDevice::KeyboardMouse => self.keyboard_mouse,
        }
    }
}

/// How many players there can be and how they join.
#[derive(Clone, PartialEq, Debug)]
pub struct JoinSettings {
    /// Indices `0..max_players` exist.
    pub max_players: u8,
    /// An unassigned pad that presses a join button joins.
    pub join_on_press: bool,
    /// The pad buttons that join.
    pub join_buttons: Vec<GamepadButton>,
    /// The unassigned keyboard joins by pressing a join key; it also needs `join_on_press`.
    pub keyboard_mouse_joins: bool,
    /// The keys that join the keyboard and mouse.
    pub join_keys: Vec<KeyCode>,
}

impl Default for JoinSettings {
    fn default() -> Self {
        Self {
            max_players: 4,
            join_on_press: false,
            join_buttons: vec![GamepadButton::South, GamepadButton::Start],
            keyboard_mouse_joins: false,
            join_keys: vec![KeyCode::Enter, KeyCode::Space],
        }
    }
}

impl JoinSettings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_players(mut self, max_players: u8) -> Self {
        self.max_players = max_players;
        self
    }

    pub fn with_join_on_press(mut self, join_on_press: bool) -> Self {
        self.join_on_press = join_on_press;
        self
    }

    pub fn with_keyboard_mouse_joins(mut self, keyboard_mouse_joins: bool) -> Self {
        self.keyboard_mouse_joins = keyboard_mouse_joins;
        self
    }

    pub fn with_join_button(mut self, button: GamepadButton) -> Self {
        self.join_buttons.push(button);
        self
    }

    /// Replaces the join buttons.
    pub fn with_join_buttons(mut self, buttons: Vec<GamepadButton>) -> Self {
        self.join_buttons = buttons;
        self
    }

    pub fn with_join_key(mut self, key: KeyCode) -> Self {
        self.join_keys.push(key);
        self
    }

    /// Replaces the join keys.
    pub fn with_join_keys(mut self, keys: Vec<KeyCode>) -> Self {
        self.join_keys = keys;
        self
    }
}

/// Why [`Players::assign`] refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssignError {
    OutOfRange { max_players: u8 },
    Taken { by: PlayerIndex },
}

impl fmt::Display for AssignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssignError::OutOfRange { max_players } => write!(
                f,
                "player index out of range: at most {max_players} players"
            ),
            AssignError::Taken { by } => {
                write!(f, "device already held by player {}", by.index())
            }
        }
    }
}

impl std::error::Error for AssignError {}

/// A change to who plays and on what.
#[derive(Event, Clone, Copy, Debug, PartialEq)]
pub enum PlayerEvent {
    Joined {
        player: PlayerIndex,
        device: PlayerDevice,
    },
    Left {
        player: PlayerIndex,
    },
    DeviceLost {
        player: PlayerIndex,
        gamepad: GamepadId,
    },
    DeviceRestored {
        player: PlayerIndex,
        gamepad: GamepadId,
    },
}

#[derive(Clone, Copy, Default)]
struct Player {
    devices: PlayerDevices,
    lost: Option<GamepadId>,
}

/// Which local player holds which pad, and the keyboard and mouse.
#[derive(Resource)]
pub struct Players {
    settings: JoinSettings,
    players: BTreeMap<PlayerIndex, Player>,
    queued: Vec<PlayerEvent>,
}

impl Players {
    pub fn new(settings: JoinSettings) -> Self {
        Self {
            settings,
            players: BTreeMap::new(),
            queued: Vec::new(),
        }
    }

    pub fn settings(&self) -> &JoinSettings {
        &self.settings
    }

    /// Lowering `max_players` keeps players already above it and only bounds later joins.
    pub fn set_settings(&mut self, settings: JoinSettings) {
        self.settings = settings;
    }

    /// Gives `device` to `player`, joining them if their place was empty.
    pub fn assign(&mut self, player: PlayerIndex, device: PlayerDevice) -> Result<(), AssignError> {
        if player.index() >= self.settings.max_players {
            return Err(AssignError::OutOfRange {
                max_players: self.settings.max_players,
            });
        }
        if let Some(by) = self.player_of(device).filter(|&by| by != player) {
            return Err(AssignError::Taken { by });
        }
        if let PlayerDevice::Gamepad(id) = device {
            self.players
                .iter_mut()
                .filter(|(other, slot)| **other != player && slot.lost == Some(id))
                .for_each(|(_, slot)| slot.lost = None);
        }
        let joined = !self.players.contains_key(&player);
        let slot = self.players.entry(player).or_default();
        let event = match device {
            PlayerDevice::Gamepad(id) => {
                let restored = slot.lost.take().is_some();
                slot.devices.gamepad = Some(id);
                restored.then_some(PlayerEvent::DeviceRestored {
                    player,
                    gamepad: id,
                })
            }
            PlayerDevice::KeyboardMouse => {
                slot.devices.keyboard_mouse = true;
                None
            }
        };
        if joined {
            self.queued.push(PlayerEvent::Joined { player, device });
        } else if let Some(event) = event {
            self.queued.push(event);
        }
        Ok(())
    }

    /// Empties `player`'s place; does nothing when it is already empty.
    pub fn unassign(&mut self, player: PlayerIndex) {
        if self.players.remove(&player).is_some() {
            self.queued.push(PlayerEvent::Left { player });
        }
    }

    /// The devices `player` holds, or `None` for an empty place.
    pub fn devices(&self, player: PlayerIndex) -> Option<PlayerDevices> {
        self.players.get(&player).map(|slot| slot.devices)
    }

    /// The joined players, in index order.
    pub fn joined(&self) -> impl Iterator<Item = PlayerIndex> + '_ {
        self.players.keys().copied()
    }

    /// The players waiting for a lost pad, in index order.
    pub fn waiting(&self) -> impl Iterator<Item = PlayerIndex> + '_ {
        self.players
            .iter()
            .filter(|(_, slot)| slot.lost.is_some())
            .map(|(player, _)| *player)
    }

    pub fn is_waiting(&self, player: PlayerIndex) -> bool {
        self.players
            .get(&player)
            .is_some_and(|slot| slot.lost.is_some())
    }

    /// The player holding `device`, if any.
    pub fn player_of(&self, device: PlayerDevice) -> Option<PlayerIndex> {
        self.players
            .iter()
            .find(|(_, slot)| slot.devices.holds(device))
            .map(|(player, _)| *player)
    }

    pub(crate) fn lose_gamepad(&mut self, gamepad: GamepadId) -> Option<PlayerIndex> {
        let player = self.player_of(PlayerDevice::Gamepad(gamepad))?;
        let slot = self.players.get_mut(&player)?;
        slot.devices.gamepad = None;
        slot.lost = Some(gamepad);
        self.queued
            .push(PlayerEvent::DeviceLost { player, gamepad });
        Some(player)
    }

    fn restore_gamepad(&mut self, player: PlayerIndex, gamepad: GamepadId) {
        if let Some(slot) = self.players.get_mut(&player) {
            slot.devices.gamepad = Some(gamepad);
            slot.lost = None;
            self.queued
                .push(PlayerEvent::DeviceRestored { player, gamepad });
        }
    }

    fn waiting_on(&self, gamepad: GamepadId) -> Option<PlayerIndex> {
        self.players
            .iter()
            .find(|(_, slot)| slot.lost == Some(gamepad))
            .map(|(player, _)| *player)
    }

    fn lowest_empty(&self) -> Option<PlayerIndex> {
        (0..self.settings.max_players)
            .map(PlayerIndex::new)
            .find(|player| !self.players.contains_key(player))
    }

    fn join(&mut self, device: PlayerDevice) {
        let place = self
            .waiting()
            .find(|player| player.index() < self.settings.max_players)
            .filter(|_| matches!(device, PlayerDevice::Gamepad(_)))
            .or_else(|| self.lowest_empty());
        if let Some(player) = place {
            let _ = self.assign(player, device);
        }
    }

    pub(crate) fn drain_events(&mut self) -> impl Iterator<Item = PlayerEvent> + '_ {
        self.queued.drain(..)
    }
}

impl Default for Players {
    fn default() -> Self {
        Self::new(JoinSettings::new())
    }
}

pub(crate) fn update_players(
    input: Res<Input>,
    mut players: ResMut<Players>,
    mut connections: EventReader<GamepadConnection>,
    mut events: EventWriter<PlayerEvent>,
) {
    for connection in connections.read() {
        match connection {
            GamepadConnection::Disconnected { id } => {
                players.lose_gamepad(*id);
            }
            GamepadConnection::Connected { id, .. } => {
                if let Some(player) = players.waiting_on(*id) {
                    players.restore_gamepad(player, *id);
                }
            }
        }
    }
    if players.settings.join_on_press {
        join_on_press(&input, &mut players);
    }
    for event in players.drain_events() {
        events.write(event);
    }
}

fn join_on_press(input: &Input, players: &mut Players) {
    let pressed: Vec<GamepadId> = input
        .gamepads()
        .filter(|pad| players.player_of(PlayerDevice::Gamepad(pad.id())).is_none())
        .filter(|pad| {
            players
                .settings
                .join_buttons
                .iter()
                .any(|button| pad.is_just_pressed(*button))
        })
        .map(|pad| pad.id())
        .collect();
    for gamepad in pressed {
        players.join(PlayerDevice::Gamepad(gamepad));
    }
    let keyboard_joins = players.settings.keyboard_mouse_joins
        && players.player_of(PlayerDevice::KeyboardMouse).is_none()
        && players
            .settings
            .join_keys
            .iter()
            .any(|key| input.is_just_pressed(PhysicalKey::Code(*key)));
    if keyboard_joins {
        players.join(PlayerDevice::KeyboardMouse);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P0: PlayerIndex = PlayerIndex::new(0);
    const P1: PlayerIndex = PlayerIndex::new(1);
    const PAD_A: GamepadId = GamepadId::new(10);
    const PAD_B: GamepadId = GamepadId::new(11);

    fn events(players: &mut Players) -> Vec<PlayerEvent> {
        players.drain_events().collect()
    }

    #[test]
    fn assign_refuses_an_index_at_or_above_max_players() {
        let mut players = Players::new(JoinSettings::new().with_max_players(2));

        let result = players.assign(PlayerIndex::new(2), PlayerDevice::Gamepad(PAD_A));

        assert_eq!(result, Err(AssignError::OutOfRange { max_players: 2 }));
        assert_eq!(players.joined().count(), 0);
        assert!(events(&mut players).is_empty());
    }

    #[test]
    fn assign_refuses_a_device_another_player_holds_naming_that_player() {
        let mut players = Players::default();
        players.assign(P0, PlayerDevice::Gamepad(PAD_A)).unwrap();
        players.assign(P0, PlayerDevice::KeyboardMouse).unwrap();

        assert_eq!(
            players.assign(P1, PlayerDevice::Gamepad(PAD_A)),
            Err(AssignError::Taken { by: P0 })
        );
        assert_eq!(
            players.assign(P1, PlayerDevice::KeyboardMouse),
            Err(AssignError::Taken { by: P0 })
        );
        assert_eq!(players.devices(P1), None);
    }

    #[test]
    fn assigning_a_second_pad_replaces_the_first_which_is_then_free() {
        let mut players = Players::default();
        players.assign(P0, PlayerDevice::Gamepad(PAD_A)).unwrap();
        events(&mut players);

        players.assign(P0, PlayerDevice::Gamepad(PAD_B)).unwrap();

        assert_eq!(players.devices(P0).unwrap().gamepad, Some(PAD_B));
        assert_eq!(players.player_of(PlayerDevice::Gamepad(PAD_A)), None);
        assert!(events(&mut players).is_empty());
        assert_eq!(players.assign(P1, PlayerDevice::Gamepad(PAD_A)), Ok(()));
    }

    #[test]
    fn unassign_frees_the_place_and_its_devices() {
        let mut players = Players::default();
        players.assign(P0, PlayerDevice::Gamepad(PAD_A)).unwrap();
        players.assign(P0, PlayerDevice::KeyboardMouse).unwrap();
        events(&mut players);

        players.unassign(P0);

        assert_eq!(players.devices(P0), None);
        assert_eq!(players.joined().count(), 0);
        assert_eq!(players.player_of(PlayerDevice::Gamepad(PAD_A)), None);
        assert_eq!(players.player_of(PlayerDevice::KeyboardMouse), None);
        assert_eq!(events(&mut players), vec![PlayerEvent::Left { player: P0 }]);
        players.unassign(P0);
        assert!(events(&mut players).is_empty());
    }

    #[test]
    fn assign_to_an_empty_place_joins() {
        let mut players = Players::default();

        players.assign(P1, PlayerDevice::KeyboardMouse).unwrap();
        players.assign(P1, PlayerDevice::Gamepad(PAD_A)).unwrap();

        assert_eq!(
            events(&mut players),
            vec![PlayerEvent::Joined {
                player: P1,
                device: PlayerDevice::KeyboardMouse
            }]
        );
        assert_eq!(players.joined().collect::<Vec<_>>(), vec![P1]);
        assert_eq!(
            players.devices(P1),
            Some(PlayerDevices {
                gamepad: Some(PAD_A),
                keyboard_mouse: true
            })
        );
    }

    #[test]
    fn a_lost_pad_makes_its_player_wait() {
        let mut players = Players::default();
        players.assign(P0, PlayerDevice::Gamepad(PAD_A)).unwrap();
        players.assign(P0, PlayerDevice::KeyboardMouse).unwrap();
        events(&mut players);

        assert_eq!(players.lose_gamepad(PAD_A), Some(P0));
        assert_eq!(players.lose_gamepad(PAD_B), None);

        assert!(players.is_waiting(P0));
        assert_eq!(players.waiting().collect::<Vec<_>>(), vec![P0]);
        assert_eq!(
            players.devices(P0),
            Some(PlayerDevices {
                gamepad: None,
                keyboard_mouse: true
            })
        );
        assert_eq!(
            events(&mut players),
            vec![PlayerEvent::DeviceLost {
                player: P0,
                gamepad: PAD_A
            }]
        );
    }

    #[test]
    fn a_pad_given_to_a_waiting_player_restores_them() {
        let mut players = Players::default();
        players.assign(P0, PlayerDevice::Gamepad(PAD_A)).unwrap();
        players.lose_gamepad(PAD_A);
        events(&mut players);

        players.assign(P0, PlayerDevice::Gamepad(PAD_B)).unwrap();

        assert!(!players.is_waiting(P0));
        assert_eq!(players.waiting().count(), 0);
        assert_eq!(
            events(&mut players),
            vec![PlayerEvent::DeviceRestored {
                player: P0,
                gamepad: PAD_B
            }]
        );
    }

    #[test]
    fn the_keyboard_given_to_a_waiting_player_leaves_them_waiting_quietly() {
        let mut players = Players::default();
        players.assign(P0, PlayerDevice::Gamepad(PAD_A)).unwrap();
        players.lose_gamepad(PAD_A);
        events(&mut players);

        players.assign(P0, PlayerDevice::KeyboardMouse).unwrap();

        assert!(players.is_waiting(P0));
        assert!(events(&mut players).is_empty());
        players.assign(P0, PlayerDevice::Gamepad(PAD_A)).unwrap();
        assert_eq!(
            events(&mut players),
            vec![PlayerEvent::DeviceRestored {
                player: P0,
                gamepad: PAD_A
            }]
        );
    }

    #[test]
    fn the_keyboard_added_to_a_player_with_a_pad_raises_nothing() {
        let mut players = Players::default();
        players.assign(P0, PlayerDevice::Gamepad(PAD_A)).unwrap();
        events(&mut players);

        players.assign(P0, PlayerDevice::KeyboardMouse).unwrap();

        assert!(events(&mut players).is_empty());
        assert_eq!(players.player_of(PlayerDevice::KeyboardMouse), Some(P0));
    }

    #[test]
    fn assigning_a_lost_pad_to_another_player_clears_the_wait_on_it() {
        let mut players = Players::default();
        players.assign(P0, PlayerDevice::Gamepad(PAD_A)).unwrap();
        players.lose_gamepad(PAD_A);
        events(&mut players);

        players.assign(P1, PlayerDevice::Gamepad(PAD_A)).unwrap();

        assert!(!players.is_waiting(P0));
        assert_eq!(players.waiting().count(), 0);
        assert_eq!(players.joined().collect::<Vec<_>>(), vec![P0, P1]);
        assert_eq!(
            events(&mut players),
            vec![PlayerEvent::Joined {
                player: P1,
                device: PlayerDevice::Gamepad(PAD_A)
            }]
        );
    }

    #[test]
    fn lowering_max_players_keeps_players_above_it() {
        let mut players = Players::default();
        players
            .assign(PlayerIndex::new(3), PlayerDevice::Gamepad(PAD_A))
            .unwrap();

        players.set_settings(JoinSettings::new().with_max_players(2));

        assert_eq!(
            players.joined().collect::<Vec<_>>(),
            vec![PlayerIndex::new(3)]
        );
    }

    #[test]
    fn a_player_above_a_lowered_max_players_cannot_be_given_a_new_device() {
        let mut players = Players::default();
        players.assign(P1, PlayerDevice::Gamepad(PAD_A)).unwrap();

        players.set_settings(JoinSettings::new().with_max_players(1));

        assert_eq!(
            players.assign(P1, PlayerDevice::Gamepad(PAD_B)),
            Err(AssignError::OutOfRange { max_players: 1 })
        );
        assert_eq!(players.devices(P1).unwrap().gamepad, Some(PAD_A));
    }
}
