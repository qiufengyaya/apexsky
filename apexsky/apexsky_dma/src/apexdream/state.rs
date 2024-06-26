use std::sync::Arc;

use bitset_core::BitSet;
use tracing::instrument;

use crate::apexdream::*;

mod buttons;
mod client_state;
mod derivative;
pub mod entities;
mod entity_list;
mod input_system;
mod itemids;
mod modifiers;
mod name_list;
mod observer_list;
mod script_data;
mod string_tables;
mod studio;

pub use self::api::Api;
use self::data::GameData;
pub use self::derivative::EstimateDerivative;
use self::entities::{PlayerEntity, WeaponXEntity};
pub use self::studio::StudioModel;

#[derive(Default)]
pub struct GameState {
    pub time: f64,
    pub client: client_state::ClientState,
    pub entity_list: entity_list::EntityList,
    pub input_system: input_system::InputSystem,
    pub string_tables: string_tables::StringTables,
    pub name_list: name_list::NameList,
    pub observer_list: observer_list::ObserverList,
    pub buttons: buttons::Buttons,
    pub script_data: script_data::ScriptNetData,
    pub items: itemids::LootItems,
    pub mods: modifiers::Modifiers,

    gamemode_buf: [u8; 16],
    gamemode_hash: u32,
}

impl GameState {
    #[instrument(skip(self, api))]
    #[inline(never)]
    pub async fn update(&mut self, api: &Api, ctx: &mut UpdateContext) {
        // Update globals and ctx
        self.client.update(api, ctx).await;

        // Update others
        tokio::join!(
            self.entity_list.update(api, ctx),
            self.input_system.update(api, ctx),
            self.string_tables.update(api, ctx),
            self.name_list.update(api, ctx),
            self.observer_list.update(api, ctx),
            self.buttons.update(api, ctx),
            self.script_data.update(api, ctx),
            self.items.update(api, ctx),
            self.mods.update(api, ctx),
        );

        // Calling `post()` to complete the update of the entities
        for i in 0..self.entity_list.entities.len() {
            // Temporarily take the entity out of the list
            if let Some(mut entity) = self
                .entity_list
                .entities
                .get_mut(i)
                .and_then(|entity| entity.take())
            {
                // // Analyze the entities for derived information
                // {
                //     let entity_ref = (&*entity).as_ref();
                //     self.items.visit(api, ctx, entity_ref);
                //     self.mods.visit(api, ctx, entity_ref);
                // }
                // Update the entities with general game state information
                entity.post(api, ctx, self);
                // Place the entity back in the list
                if let Some(place @ &mut None) = self.entity_list.entities.get_mut(i) {
                    *place = Some(entity);
                }
            }
        }

        if ctx.connected && ctx.data.mp_gamemode != 0 {
            self.gamemode_hash = 0;
            if let Ok(gamemode_ptr) = api
                .vm_read::<sdk::Ptr<[u8]>>(api.apex_base.field(ctx.data.mp_gamemode + 0x58))
                .await
            {
                if !gamemode_ptr.is_null() {
                    if let Ok(gamemode) =
                        api.vm_read_cstr(gamemode_ptr, &mut self.gamemode_buf).await
                    {
                        self.gamemode_hash = crate::apexdream::base::hash(gamemode);
                    }
                }
            }
        }
    }
    pub fn gamemode(&self) -> Option<&str> {
        base::from_utf8_buf(&self.gamemode_buf)
    }
    /// Returns if the players are on the same team.
    /// This matters in control mode where multiple squads can be in the same team.
    pub fn is_same_team(&self, p1: &PlayerEntity, p2: &PlayerEntity) -> bool {
        if self.gamemode_hash == sdk::GAMEMODE_CONTROL || self.gamemode_hash == sdk::GAMEMODE_FREEDM
        {
            p1.team_num & 1 == p2.team_num & 1
        } else {
            p1.team_num == p2.team_num
        }
    }
}

//----------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub struct ValueChanged<T> {
    pub time: f64,
    pub old: T,
    pub new: T,
}
impl<T> ValueChanged<T> {
    pub const fn new(time: f64, old: T, new: T) -> ValueChanged<T> {
        ValueChanged { time, old, new }
    }
}

//----------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct UpdateContext {
    pub data: Arc<GameData>,

    pub time: f64,
    pub tickcount: u32,

    // Connection state changed to fully connected
    pub connected: bool,
    // Prioritize updating local player related information
    pub local_entity: sdk::EHandle,
    // Update full bones instead of only spine
    pub full_bones: bool,
}

impl UpdateContext {
    /// Rate limit reading less important variables.
    #[inline]
    pub fn ticked(&self, rate: u32, offset: u32) -> bool {
        if rate <= 1 {
            return true;
        }
        (self.tickcount.wrapping_add(offset)) % rate == 0
    }
}

//----------------------------------------------------------------

impl GameState {
    pub fn get_fov(&self, player: &PlayerEntity) -> f32 {
        if player.zooming {
            if let Some(weapon) = self.entity_as::<WeaponXEntity>(player.active_weapon) {
                if weapon.target_zoom_fov > 1.0 && weapon.target_zoom_fov <= 90.0 {
                    return weapon.target_zoom_fov;
                }
            }
        }
        return 90.0;
    }
    pub fn desired_items(&self, player: &PlayerEntity) -> sdk::ItemSet {
        // Start by collecting desired items from the player
        let mut desired_items = player.desired_items(Some(self));

        // Add the set of desired items from their primary and secondary weapon
        if let Some(weapon) = self.entity_as::<WeaponXEntity>(player.weapons[0]) {
            desired_items.bit_or(&weapon.desired_items(self));
        }
        if let Some(weapon) = self.entity_as::<WeaponXEntity>(player.weapons[1]) {
            desired_items.bit_or(&weapon.desired_items(self));
        }
        return desired_items;
    }
    pub fn player_is_melee(&self, player: &PlayerEntity) -> bool {
        if let Some(weapon) = player.active_weapon(self) {
            self.weapon_is_melee(weapon.weapon_name_index)
                || weapon.weapon_name == sdk::WeaponName::CONSUMABLE
        } else {
            true
        }
    }
}
