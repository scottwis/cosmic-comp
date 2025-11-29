# Cosmic Comp Overview

## Project Purpose

`cosmic-comp` is the Wayland compositor that powers the COSMIC desktop. It is
built on top of Smithay's compositor toolkit and integrates deeply with other
COSMIC services to provide window management, input handling, rendering, and
desktop shell features. The repository contains the main compositor crate and a
companion configuration crate that describe user and system preferences.

## Runtime Entry Points and Control Flow

- **`src/main.rs`**: Minimal executable that delegates to `cosmic_comp::run`.
- **`src/lib.rs`**: Exposes `run`, the primary entry point. It parses CLI
  arguments, sets up logging and profiling, initializes the Wayland display and
  event loop, and constructs the global [`State`](src/state.rs). Runtime hooks
  are registered here and the appropriate backend is started before the event
  loop is executed.
- **Event loop**: A `calloop` loop drives state updates. Each iteration checks
  shutdown flags, advances shell animations, refreshes outputs, flushes Wayland
  clients, and supervises kiosk-mode child processes if one was spawned.

## Core Crates and Modules

### `cosmic-comp`

- **`backend/`**: Chooses and initializes the rendering/input backend. It
  provides implementations for:
  - `kms`: Direct rendering on devices that support KMS/DRM.
  - `winit`: Windowed backend for development environments.
  - `x11`: XWayland-based fallback. All backends feed into a common
    initialization path that creates the first seat and applies accessibility
    preferences via Smithay.
  - `render/`: Helpers for renderer selection, cursor management, and GPU
    resources, used by multiple backends.

- **`state.rs`**: Defines the compositor's master state object. It aggregates:
  - Backend-specific state (KMS, Winit, X11, XWayland).
  - Wayland protocol state machines (damage tracking, dmabuf, presentation,
    shell layers, etc.).
  - Workspace, accessibility, input, and rendering metadata.
  - Utilities for notifying readiness, scheduling renders, and propagating
    configuration changes.

- **`shell/`**: Implements the COSMIC window management model. Submodules cover
  window/surface abstractions, tiling and layout logic, seat handling,
  workspace transitions, zoom functionality, and focus management. The shell is
  responsible for creating seats, tracking outputs, and coordinating animations.

- **`wayland/`**: Split into two layers:
  - `protocols/`: Definitions and state wrappers for COSMIC-specific Wayland
    protocols (e.g., accessibility, corner radius, workspace management).
  - `handlers/`: Bindings that register those protocols with Smithay and
    translate incoming Wayland requests into shell or state updates.

- **`input/`**: Centralizes input device handling. Provides configuration-aware
  routing for keyboard, pointer, and touch events, plus gesture recognition.

- **`config/`**: Loads and applies compositor and input settings, including key
  bindings, output preferences, and screen filters. It bridges serialized
  configuration data with runtime behavior.

- **`dbus/`, `session.rs`, `systemd.rs`**: Integrations with external system
  services. They update the D-Bus activation environment, coordinate with the
  `cosmic-session` process over a Unix socket, and signal readiness to systemd
  units when enabled.

- **`xwayland.rs`**: Manages the optional XWayland server, including window
  manager integration, clipboard synchronization, and cursor handling.

- **`theme.rs`**: Watches COSMIC theme settings via `cosmic-config`, updates the
  compositor's active theme, and triggers redraws when colors or modes change.

- **`hooks.rs` & `debug.rs`**: Provide optional customization hooks for window
  decorations and debugging utilities that are enabled via compile-time
  features.

- **`utils/`**: Shared helpers for geometry, ID generation, tweening, resource
  limits, and environment handling. `utils::prelude` re-exports commonly used
  types across modules.

- **`logger/`**: Configures tracing subscribers and optional journald logging so
  compositor logs integrate with system logging.

### `cosmic-comp-config`

This workspace member crate defines the serialized configuration schema used by
the compositor. It includes:

- Core `CosmicCompConfig` settings (workspaces, input defaults, autotiling,
  accessibility options, XKB settings).
- Input device overrides (`input.rs`), output-specific configuration (`output/`)
  behind the `output` feature, and workspace pinning metadata (`workspace.rs`).
- Conversions for EDID metadata when `libdisplay-info` is enabled, allowing the
  compositor to match physical displays with stored preferences.

## Assets, Packaging, and Tooling

- **`resources/`**: Assets embedded into the binary via `rust-embed`, including
  i18n strings, icons, custom cursors, and shipped Wayland protocol XML files.
- **`data/`**: Distribution artifacts such as systemd units, desktop entries,
  and default RON configuration files consumed at runtime.
- **`examples/`**: Sample code (e.g., custom window decorations) demonstrating
  how to use the compositor's hook interfaces from external crates.
- **`build.rs`**: Captures the repository's current Git hash and exports it as a
  compile-time environment variable, used by `cosmic_comp --version`.
- **`Makefile`, `flake.nix`, `debian/`**: Provide convenience targets, Nix
  packaging definitions, and Debian packaging metadata for building and
  distributing the compositor.

## Putting It All Together

At startup, `cosmic-comp` establishes logging, parses configuration, and
initializes the event loop and Smithay display. The chosen backend sets up
rendering and input pipelines, after which the shell constructs seats, outputs,
and workspaces. Wayland protocol handlers mediate client interactions while
system integration modules synchronize environment state with systemd, D-Bus,
and cosmic-session. Runtime configuration and theme watchers feed changes back
into the shell, ensuring the compositor reflects user preferences without
restarts. The supporting configuration crate, assets, and packaging files make
the compositor configurable and distributable across different environments.

