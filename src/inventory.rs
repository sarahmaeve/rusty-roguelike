use bevy::prelude::*;

use crate::components::ItemKind;

// ── Events ─────────────────────────────────────────────────────────────────────

/// Fired when the player activates an inventory item on a world tile.
/// The `item` is the kind being used; `target` is the grid position.
/// Systems in `PlayerPlugin` listen for this to apply the game-logic effect.
#[derive(Event, Clone, Copy)]
pub struct UseItemEvent {
    pub item: ItemKind,
    pub target: (i32, i32),
}

// ── Tunables ──────────────────────────────────────────────────────────────────

pub const SLOT_COUNT: usize = 5;

const SLOT_SIZE: f32 = 60.0;
const SLOT_GAP:  f32 = 8.0;

// Palette matching the existing HUD (warm gold on dark).
const FG:      (f32, f32, f32) = (0.95, 0.85, 0.55);
const SLOT_BG: (f32, f32, f32, f32) = (0.05, 0.05, 0.12, 0.85);

// ── Resource ──────────────────────────────────────────────────────────────────

/// The player's carried items.  Capped at [`SLOT_COUNT`] entries.
#[derive(Resource, Default)]
pub struct Inventory {
    items: Vec<ItemKind>,
}

impl Inventory {
    /// Adds `item` and returns `true`, or returns `false` if the inventory is full.
    pub fn add(&mut self, item: ItemKind) -> bool {
        if self.items.len() < SLOT_COUNT {
            self.items.push(item);
            true
        } else {
            false
        }
    }

    pub fn items(&self) -> &[ItemKind] {
        &self.items
    }

    /// Removes the first occurrence of `item` and returns `true`, or `false`
    /// if the item was not present.
    pub fn remove(&mut self, item: ItemKind) -> bool {
        if let Some(pos) = self.items.iter().position(|&k| k == item) {
            self.items.remove(pos);
            true
        } else {
            false
        }
    }
}

// ── Selected-slot resource ─────────────────────────────────────────────────────

/// Tracks which inventory slot (if any) the player has clicked to activate.
/// When `Some(i)`, the next world left-click fires [`UseItemEvent`] with that
/// item, then clears the selection.  Press **Escape** or right-click to cancel.
#[derive(Resource, Default)]
pub struct SelectedSlot(pub Option<usize>);

// ── HUD components ────────────────────────────────────────────────────────────

/// Marks the inventory HUD root node.
#[derive(Component)]
struct InventoryRoot;

/// Marks a slot container node; the inner `usize` is the slot index (0-based).
#[derive(Component)]
struct InventorySlot(usize);

/// Marks a slot label; the inner `usize` is the slot index (0-based).
#[derive(Component)]
struct InventorySlotLabel(usize);

// ── Startup system ────────────────────────────────────────────────────────────

fn spawn_inventory_hud(mut commands: Commands) {
    // Full-width absolute row anchored to the bottom; flex centres the slots.
    commands
        .spawn((
            InventoryRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(20.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(SLOT_GAP),
                ..Default::default()
            },
        ))
        .with_children(|row| {
            for i in 0..SLOT_COUNT {
                row.spawn((
                    InventorySlot(i),
                    Interaction::default(),
                    Node {
                        width:           Val::Px(SLOT_SIZE),
                        height:          Val::Px(SLOT_SIZE),
                        justify_content: JustifyContent::Center,
                        align_items:     AlignItems::Center,
                        ..Default::default()
                    },
                    BackgroundColor(Color::srgba(
                        SLOT_BG.0, SLOT_BG.1, SLOT_BG.2, SLOT_BG.3,
                    )),
                    BorderRadius::all(Val::Px(6.0)),
                ))
                .with_children(|slot| {
                    slot.spawn((
                        InventorySlotLabel(i),
                        Text::new(""),
                        TextFont { font_size: 13.0, ..Default::default() },
                        TextColor(Color::srgba(FG.0, FG.1, FG.2, 0.9)),
                    ));
                });
            }
        });
}

// ── Update system ─────────────────────────────────────────────────────────────

fn update_inventory_hud(
    inventory: Res<Inventory>,
    mut label_q: Query<(&InventorySlotLabel, &mut Text)>,
) {
    if !inventory.is_changed() {
        return;
    }
    for (InventorySlotLabel(i), mut text) in &mut label_q {
        **text = inventory
            .items()
            .get(*i)
            .map_or(String::new(), |k| k.display_name().to_string());
    }
}

// ── Update system: slot selection ─────────────────────────────────────────────

const SLOT_BG_SELECTED: (f32, f32, f32, f32) = (0.45, 0.30, 0.05, 0.95);

/// Click a filled slot to select it (click again to deselect).
/// Only slots that hold an item can be selected.
fn select_inventory_slot(
    mut interaction_q: Query<
        (&Interaction, &InventorySlot),
        Changed<Interaction>,
    >,
    inventory: Res<Inventory>,
    mut selected_slot: ResMut<SelectedSlot>,
) {
    for (interaction, InventorySlot(i)) in &mut interaction_q {
        if *interaction == Interaction::Pressed && inventory.items().get(*i).is_some() {
            selected_slot.0 = if selected_slot.0 == Some(*i) { None } else { Some(*i) };
        }
    }
}

/// Update slot background colours to reflect the current selection.
fn update_slot_highlight(
    selected_slot: Res<SelectedSlot>,
    mut slot_q: Query<(&InventorySlot, &mut BackgroundColor)>,
) {
    if !selected_slot.is_changed() {
        return;
    }
    for (InventorySlot(i), mut bg) in &mut slot_q {
        *bg = if selected_slot.0 == Some(*i) {
            BackgroundColor(Color::srgba(
                SLOT_BG_SELECTED.0, SLOT_BG_SELECTED.1,
                SLOT_BG_SELECTED.2, SLOT_BG_SELECTED.3,
            ))
        } else {
            BackgroundColor(Color::srgba(SLOT_BG.0, SLOT_BG.1, SLOT_BG.2, SLOT_BG.3))
        };
    }
}

/// Press **Escape** or right-click to cancel item selection.
fn cancel_item_selection(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut selected_slot: ResMut<SelectedSlot>,
) {
    if (keyboard.just_pressed(KeyCode::Escape) || mouse.just_pressed(MouseButton::Right))
        && selected_slot.0.is_some()
    {
        selected_slot.0 = None;
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct InventoryPlugin;

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Inventory>()
            .init_resource::<SelectedSlot>()
            .add_event::<UseItemEvent>()
            .add_systems(Startup, spawn_inventory_hud)
            .add_systems(
                Update,
                (
                    update_inventory_hud,
                    select_inventory_slot,
                    update_slot_highlight,
                    cancel_item_selection,
                ),
            );
    }
}
