// SPDX-License-Identifier: GPL-3.0-only

use super::Workspace;
use crate::{
    shell::{
        element::{CosmicMapped, MaximizedState},
        layout::{floating::TiledCorners, tiling::Data},
        ManagedLayer, MinimizedWindow,
        CosmicSurface,
    },
    state::State,
    wayland::protocols::{
        toplevel_info::{toplevel_enter_output, toplevel_enter_workspace},
        workspace::WorkspaceUpdateGuard,
    },
};
use id_tree::Tree;
use smithay::{
    input::Seat,
    utils::{IsAlive, Local, Rectangle},
};
use std::{collections::HashSet, time::Duration};

const MAX_HISTORY: usize = 64;

#[derive(Debug, Clone)]
pub(super) struct FloatingWindowSnapshot {
    pub window: CosmicMapped,
    pub geometry: Rectangle<i32, Local>,
    pub tiled_corners: Option<TiledCorners>,
    pub was_maximized: bool,
}

#[derive(Debug, Clone)]
pub(super) struct FullscreenSnapshot {
    pub surface: CosmicSurface,
    pub previous_state: Option<super::FullscreenRestoreState>,
    pub previous_geometry: Option<Rectangle<i32, Local>>,
}

#[derive(Debug, Clone)]
pub(super) struct WorkspaceLayoutSnapshot {
    pub tiling_enabled: bool,
    pub tiling_tree: Tree<Data>,
    pub floating_windows: Vec<FloatingWindowSnapshot>,
    pub fullscreen: Option<FullscreenSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SnapshotError {
    MissingWindow,
}

#[derive(Debug, Default)]
pub(super) struct LayoutHistory {
    snapshots: Vec<WorkspaceLayoutSnapshot>,
    current: usize,
    dirty: bool,
    suspend: u32,
}

impl LayoutHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn initialize(&mut self, snapshot: WorkspaceLayoutSnapshot) {
        self.snapshots.clear();
        self.snapshots.push(snapshot);
        self.current = 0;
        self.dirty = false;
        self.suspend = 0;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn maybe_record(&mut self, workspace: &Workspace, tiling_dirty: bool) {
        if self.suspend > 0 {
            return;
        }

        if !(tiling_dirty || self.dirty) {
            return;
        }

        let snapshot = workspace.capture_layout_snapshot();

        if !self.snapshots.is_empty() && self.current + 1 < self.snapshots.len() {
            self.snapshots.truncate(self.current + 1);
        }

        self.snapshots.push(snapshot);
        if self.snapshots.len() > MAX_HISTORY {
            let remove = self.snapshots.len() - MAX_HISTORY;
            self.snapshots.drain(0..remove);
            self.current = self.current.saturating_sub(remove);
        }

        self.current = self.snapshots.len().saturating_sub(1);
        self.dirty = false;
    }

    pub fn undo(
        &mut self,
        workspace: &mut Workspace,
        seat: &Seat<State>,
        workspace_state: &mut WorkspaceUpdateGuard<'_, State>,
    ) -> bool {
        if self.snapshots.is_empty() || self.current == 0 {
            return false;
        }

        let mut idx = self.current - 1;
        loop {
            let snapshot = self.snapshots[idx].clone();
            self.suspend += 1;
            let result = workspace.apply_layout_snapshot(&snapshot, seat, workspace_state);
            self.suspend -= 1;

            match result {
                Ok(()) => {
                    self.current = idx;
                    self.dirty = false;
                    return true;
                }
                Err(SnapshotError::MissingWindow) => {
                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                }
            }
        }

        false
    }

    pub fn redo(
        &mut self,
        workspace: &mut Workspace,
        seat: &Seat<State>,
        workspace_state: &mut WorkspaceUpdateGuard<'_, State>,
    ) -> bool {
        if self.snapshots.is_empty() || self.current + 1 >= self.snapshots.len() {
            return false;
        }

        let mut idx = self.current + 1;
        while idx < self.snapshots.len() {
            let snapshot = self.snapshots[idx].clone();
            self.suspend += 1;
            let result = workspace.apply_layout_snapshot(&snapshot, seat, workspace_state);
            self.suspend -= 1;

            match result {
                Ok(()) => {
                    self.current = idx;
                    self.dirty = false;
                    return true;
                }
                Err(SnapshotError::MissingWindow) => {
                    idx += 1;
                }
            }
        }

        false
    }
}

impl Workspace {
    pub(super) fn initialize_layout_history(&mut self) {
        let snapshot = self.capture_layout_snapshot();
        self.layout_history.initialize(snapshot);
    }

    pub(super) fn capture_layout_snapshot(&self) -> WorkspaceLayoutSnapshot {
        let tiling_tree = self.tiling_layer.tree().copy_clone();

        let floating_windows = self
            .floating_layer
            .mapped()
            .filter_map(|window| {
                let geometry = self.floating_layer.element_geometry(window)?;
                Some(FloatingWindowSnapshot {
                    window: window.clone(),
                    geometry,
                    tiled_corners: window.floating_tiled.lock().unwrap().clone(),
                    was_maximized: window.is_maximized(false),
                })
            })
            .collect::<Vec<_>>();

        let fullscreen = self
            .fullscreen
            .as_ref()
            .filter(|f| f.ended_at.is_none())
            .map(|f| FullscreenSnapshot {
                surface: f.surface.clone(),
                previous_state: f.previous_state.clone(),
                previous_geometry: f.previous_geometry,
            });

        WorkspaceLayoutSnapshot {
            tiling_enabled: self.tiling_enabled,
            tiling_tree,
            floating_windows,
            fullscreen,
        }
    }

    pub(super) fn apply_layout_snapshot(
        &mut self,
        snapshot: &WorkspaceLayoutSnapshot,
        seat: &Seat<State>,
        workspace_state: &mut WorkspaceUpdateGuard<'_, State>,
    ) -> Result<(), SnapshotError> {
        if let Some(root_id) = snapshot.tiling_tree.root_node_id() {
            for node_id in snapshot
                .tiling_tree
                .traverse_pre_order_ids(root_id)
                .unwrap()
            {
                if let Data::Mapped { mapped, .. } = snapshot.tiling_tree.get(&node_id).unwrap().data() {
                    if !mapped.alive() {
                        return Err(SnapshotError::MissingWindow);
                    }
                }
            }
        }

        for floating in &snapshot.floating_windows {
            if !floating.window.alive() {
                return Err(SnapshotError::MissingWindow);
            }
        }

        if let Some(fullscreen) = &snapshot.fullscreen {
            if !fullscreen.surface.alive() {
                return Err(SnapshotError::MissingWindow);
            }
        }

        let mut target_windows: HashSet<CosmicMapped> = HashSet::new();

        if let Some(root_id) = snapshot.tiling_tree.root_node_id() {
            for node_id in snapshot
                .tiling_tree
                .traverse_pre_order_ids(root_id)
                .unwrap()
            {
                if let Data::Mapped { mapped, .. } = snapshot.tiling_tree.get(&node_id).unwrap().data() {
                    target_windows.insert(mapped.clone());
                }
            }
        }

        for floating in &snapshot.floating_windows {
            target_windows.insert(floating.window.clone());
        }

        let current_fullscreen = self
            .fullscreen
            .as_ref()
            .filter(|f| f.ended_at.is_none())
            .map(|f| f.surface.clone());

        let target_fullscreen_surface = snapshot.fullscreen.as_ref().map(|f| f.surface.clone());
        if current_fullscreen != target_fullscreen_surface {
            let _ = self.remove_fullscreen();
        }

        let current_tiling = self
            .tiling_layer
            .mapped()
            .map(|(w, _)| w.clone())
            .collect::<Vec<_>>();
        for window in current_tiling {
            let _ = self.tiling_layer.unmap(&window, None);
            if !target_windows.contains(&window) && window.alive() {
                window.send_close();
            }
        }

        let current_floating = self
            .floating_layer
            .mapped()
            .cloned()
            .collect::<Vec<_>>();
        for window in current_floating {
            let _ = self.floating_layer.unmap(&window, None);
            if !target_windows.contains(&window) && window.alive() {
                window.send_close();
            }
        }

        self.minimized_windows
            .retain(|min| min.mapped().is_some_and(|m| target_windows.contains(m)));

        for floating in &snapshot.floating_windows {
            let window = floating.window.clone();
            if !window.alive() {
                return Err(SnapshotError::MissingWindow);
            }

            window.set_minimized(false);
            window.set_tiled(false);
            window.set_maximized(false);

            for (surface, _) in window.windows() {
                toplevel_enter_output(&surface, &self.output);
                toplevel_enter_workspace(&surface, &self.handle);
            }

            if floating.was_maximized {
                let mut state = window.maximized_state.lock().unwrap();
                *state = Some(MaximizedState {
                    original_geometry: floating.geometry,
                    original_layer: ManagedLayer::Floating,
                });
                drop(state);
                self.floating_layer
                    .map_maximized(window.clone(), floating.geometry, false);
            } else {
                self.floating_layer.map_internal(
                    window.clone(),
                    Some(floating.geometry.loc),
                    Some(floating.geometry.size.as_logical()),
                    None,
                );
                window.set_maximized(false);
            }

            *window.floating_tiled.lock().unwrap() = floating.tiled_corners;
        }

        if let Some(root_id) = snapshot.tiling_tree.root_node_id() {
            for node_id in snapshot
                .tiling_tree
                .traverse_pre_order_ids(root_id)
                .unwrap()
            {
                if let Data::Mapped { mapped, .. } = snapshot.tiling_tree.get(&node_id).unwrap().data() {
                    for (surface, _) in mapped.windows() {
                        toplevel_enter_output(&surface, &self.output);
                        toplevel_enter_workspace(&surface, &self.handle);
                    }
                }
            }
        }

        let tiling_tree = snapshot.tiling_tree.copy_clone();
        self.tiling_layer.replace_tree(tiling_tree);
        self.tiling_layer.clear_history_dirty();

        if self.tiling_enabled != snapshot.tiling_enabled {
            self.tiling_enabled = snapshot.tiling_enabled;
            workspace_state.set_workspace_tiling_state(
                &self.handle,
                if self.tiling_enabled {
                    super::TilingState::TilingEnabled
                } else {
                    super::TilingState::FloatingOnly
                },
            );
        }

        if let Some(fullscreen) = &snapshot.fullscreen {
            if fullscreen.surface.alive() {
                self.map_fullscreen(
                    &fullscreen.surface,
                    Some(seat),
                    fullscreen.previous_state.clone(),
                    fullscreen.previous_geometry,
                );
            }
        }

        self.refresh_focus_stack();
        self.dirty.store(true, std::sync::atomic::Ordering::SeqCst);

        Ok(())
    }
}
