use crate::{
    apu::Channel,
    common::{Kind, NesRegion, Reset},
    cpu::{
        instr::{Instr, Operation},
        Cpu,
    },
    input::{JoypadBtn, JoypadBtnState, Slot},
    mapper::MapperRevision,
    mem::{Access, Mem},
    nes::{menu::Menu, Mode, Nes, NesResult, ReplayMode, NES_FRAME_SRC},
    video::VideoFilter,
};
use pix_engine::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashMap,
    fmt,
    ops::{Deref, DerefMut},
    time::{Duration, Instant},
};

/// Indicates an [Axis] direction.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[must_use]
pub(crate) enum AxisDirection {
    /// No direction, axis is in a deadzone/not pressed.
    None,
    /// Positive (Right or Down)
    Positive,
    /// Negative (Left or Up)
    Negative,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[must_use]
pub(crate) struct ActionEvent {
    pub(crate) frame: u32,
    pub(crate) slot: Slot,
    pub(crate) action: Action,
    pub(crate) pressed: bool,
    pub(crate) repeat: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[must_use]
pub(crate) enum Input {
    Key((Slot, Key, KeyMod)),
    Button((Slot, ControllerButton)),
    Axis((Slot, Axis, AxisDirection)),
    Mouse((Slot, Mouse)),
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Input::Key((_, key, keymod)) => {
                if keymod.is_empty() {
                    write!(f, "{key:?}")
                } else {
                    write!(f, "{keymod:?} {key:?}")
                }
            }
            Input::Button((_, btn)) => write!(f, "{btn:?}"),
            Input::Axis((_, axis, _)) => write!(f, "{axis:?}"),
            Input::Mouse((_, btn)) => write!(f, "{btn:?}"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct KeyBinding {
    pub(crate) player: Slot,
    pub(crate) key: Key,
    pub(crate) keymod: KeyMod,
    pub(crate) action: Action,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct MouseBinding {
    pub(crate) player: Slot,
    pub(crate) button: Mouse,
    pub(crate) action: Action,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct ControllerButtonBinding {
    pub(crate) player: Slot,
    pub(crate) button: ControllerButton,
    pub(crate) action: Action,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct ControllerAxisBinding {
    pub(crate) player: Slot,
    pub(crate) axis: Axis,
    pub(crate) direction: AxisDirection,
    pub(crate) action: Action,
}

/// A binding of a inputs to an [Action].
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct InputBindings {
    pub(crate) keys: Vec<KeyBinding>,
    pub(crate) mouse: Vec<MouseBinding>,
    pub(crate) buttons: Vec<ControllerButtonBinding>,
    pub(crate) axes: Vec<ControllerAxisBinding>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputMapping(HashMap<Input, Action>);

impl Deref for InputMapping {
    type Target = HashMap<Input, Action>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for InputMapping {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[allow(variant_size_differences)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum Action {
    Nes(NesState),
    Menu(Menu),
    Feature(Feature),
    Setting(Setting),
    Joypad(JoypadBtn),
    ZapperTrigger,
    ZeroAxis([JoypadBtn; 2]),
    Debug(DebugAction),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum NesState {
    Quit,
    TogglePause,
    SoftReset,
    HardReset,
    MapperRevision(MapperRevision),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum Feature {
    ToggleGameplayRecording,
    ToggleSoundRecording,
    Rewind,
    TakeScreenshot,
    SaveState,
    LoadState,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum Setting {
    SetSaveSlot(u8),
    ToggleFullscreen,
    ToggleVsync,
    ToggleNtscFilter,
    SetVideoFilter(VideoFilter),
    SetNesFormat(NesRegion),
    ToggleSound,
    TogglePulse1,
    TogglePulse2,
    ToggleTriangle,
    ToggleNoise,
    ToggleDmc,
    FastForward,
    IncSpeed,
    DecSpeed,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum DebugAction {
    ToggleCpuDebugger,
    TogglePpuDebugger,
    ToggleApuDebugger,
    StepInto,
    StepOver,
    StepOut,
    StepFrame,
    StepScanline,
    IncScanline,
    DecScanline,
}

fn render_message(s: &mut PixState, message: &str, color: Color) -> NesResult<()> {
    s.push();
    s.stroke(None);
    s.fill(rgb!(0, 200));
    let pady = s.theme().spacing.frame_pad.y();
    let width = s.width()?;
    s.wrap(width);
    let (_, height) = s.size_of(message)?;
    s.rect([
        0,
        s.cursor_pos().y() - pady,
        width as i32,
        height as i32 + 2 * pady,
    ])?;
    s.fill(color);
    s.text(message)?;
    s.pop();
    Ok(())
}

impl Nes {
    #[inline]
    pub(crate) fn add_message<S>(&mut self, text: S)
    where
        S: Into<String>,
    {
        let text = text.into();
        self.messages.push((text, Instant::now()));
    }

    pub(crate) fn render_messages(&mut self, s: &mut PixState) -> NesResult<()> {
        self.messages
            .retain(|(_, created)| created.elapsed() < Duration::from_secs(3));
        self.messages.dedup_by(|a, b| a.0.eq(&b.0));
        for (message, _) in &self.messages {
            render_message(s, message, Color::WHITE)?;
        }
        Ok(())
    }

    pub(crate) fn render_confirm_quit(&mut self, s: &mut PixState) -> NesResult<bool> {
        if let Some((ref msg, ref mut confirm)) = self.confirm_quit {
            s.push();
            s.stroke(None);
            s.fill(rgb!(0, 200));
            let pady = s.theme().spacing.frame_pad.y();
            let width = s.width()?;
            s.wrap(width);
            let (_, height) = s.size_of(msg)?;
            s.rect([
                0,
                s.cursor_pos().y() - pady,
                width as i32,
                4 * height as i32 + 2 * pady,
            ])?;
            s.fill(Color::WHITE);
            s.text(msg)?;
            if s.button("Confirm")? {
                *confirm = true;
                s.pop();
                return Ok(true);
            }
            s.same_line(None);
            if s.button("Cancel")? {
                self.confirm_quit = None;
                self.resume_play();
            }
            s.pop();
        }
        Ok(false)
    }

    #[inline]
    pub(crate) fn render_status(&mut self, s: &mut PixState, status: &str) -> PixResult<()> {
        render_message(s, status, Color::WHITE)?;
        if let Some(ref err) = self.error {
            render_message(s, err, Color::RED)?;
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn handle_input(
        &mut self,
        s: &mut PixState,
        slot: Slot,
        input: Input,
        pressed: bool,
        repeat: bool,
    ) -> NesResult<bool> {
        self.config
            .input_map
            .get(&input)
            .copied()
            .map_or(Ok(false), |action| {
                self.handle_action(s, slot, action, pressed, repeat)
            })
    }

    pub(crate) fn handle_key_event(
        &mut self,
        s: &mut PixState,
        event: KeyEvent,
        pressed: bool,
    ) -> bool {
        for slot in [Slot::One, Slot::Two, Slot::Three, Slot::Four] {
            let input = Input::Key((slot, event.key, event.keymod));
            if slot == Slot::Three {
                println!("{input:?}");
            }
            if let Ok(true) = self.handle_input(s, slot, input, pressed, event.repeat) {
                return true;
            }
        }
        false
    }

    pub fn handle_mouse_click(&mut self, s: &mut PixState, btn: Mouse) -> bool {
        // To avoid consuming events while in menus
        if self.mode == Mode::Playing {
            for slot in [Slot::One, Slot::Two] {
                let input = Input::Mouse((slot, btn));
                if let Ok(true) = self.handle_input(s, slot, input, true, false) {
                    return true;
                }
            }
        }
        false
    }

    #[inline]
    fn handle_zapper_trigger(&mut self) {
        self.control_deck.trigger_zapper();
    }

    pub fn set_zapper_pos(&mut self, pos: Point<i32>) {
        let mut pos = pos / self.config.scale as i32;
        pos.set_x((pos.x() as f32 * 8.0 / 7.0 + 0.5) as i32); // Adjust ratio
        if pos.y() < NES_FRAME_SRC.top() {
            pos.set_y(8);
        } else if pos.y() >= NES_FRAME_SRC.bottom() {
            pos.set_y(NES_FRAME_SRC.bottom() - 1);
        }
        self.control_deck.aim_zapper(pos.x(), pos.y());
    }

    #[inline]
    pub fn handle_mouse_motion(&mut self, pos: Point<i32>) -> bool {
        // To avoid consuming events while in menus
        if self.mode == Mode::Playing {
            self.set_zapper_pos(pos);
            true
        } else {
            false
        }
    }

    #[inline]
    pub(crate) fn handle_controller_event(
        &mut self,
        s: &mut PixState,
        event: ControllerEvent,
        pressed: bool,
    ) -> PixResult<bool> {
        self.get_controller_slot(event.controller_id)
            .map_or(Ok(false), |slot| {
                let input = Input::Button((slot, event.button));
                self.handle_input(s, slot, input, pressed, false)
            })
    }

    #[inline]
    pub(crate) fn handle_controller_axis(
        &mut self,
        s: &mut PixState,
        controller_id: ControllerId,
        axis: Axis,
        value: i32,
    ) -> PixResult<bool> {
        self.get_controller_slot(controller_id)
            .map_or(Ok(false), |slot| {
                let direction = match value.cmp(&0) {
                    Ordering::Greater => AxisDirection::Positive,
                    Ordering::Less => AxisDirection::Negative,
                    Ordering::Equal => AxisDirection::None,
                };
                let input = Input::Axis((slot, axis, direction));
                self.handle_input(s, slot, input, true, false)
            })
    }

    pub(crate) fn handle_action(
        &mut self,
        s: &mut PixState,
        slot: Slot,
        action: Action,
        pressed: bool,
        repeat: bool,
    ) -> PixResult<bool> {
        let handled = match action {
            Action::Debug(action) if pressed => self.handle_debug(s, action, repeat)?,
            Action::Feature(feature) => {
                self.handle_feature(s, feature, pressed, repeat);
                true
            }
            Action::Nes(state) if pressed => self.handle_nes_state(s, state)?,
            Action::Menu(menu) if pressed => {
                self.toggle_menu(s, menu)?;
                true
            }
            Action::Setting(setting) => self.handle_setting(s, setting, pressed, repeat)?,
            Action::Joypad(button) => self.handle_joypad_pressed(slot, button, pressed),
            Action::ZapperTrigger if pressed => {
                self.handle_zapper_trigger();
                true
            }
            Action::ZeroAxis(buttons) => {
                let mut handled = false;
                for button in buttons {
                    if self.handle_joypad_pressed(slot, button, pressed) {
                        handled = true;
                        break;
                    }
                }
                handled
            }
            _ => false,
        };

        if !repeat {
            log::trace!(
                "Input: {{ action: {:?}, slot: {:?}, pressed: {}, handled: {} }}",
                action,
                slot,
                pressed,
                handled,
            );
        }

        if self.replay.mode == ReplayMode::Recording {
            self.replay
                .buffer
                .push(self.action_event(slot, action, pressed, repeat));
        }

        Ok(handled)
    }

    pub(crate) fn replay_action(&mut self, s: &mut PixState) -> NesResult<()> {
        let current_frame = self.control_deck.frame_number();
        while let Some(action_event) = self.replay.buffer.last() {
            match action_event.frame.cmp(&current_frame) {
                Ordering::Equal => {
                    let ActionEvent {
                        slot,
                        action,
                        pressed,
                        repeat,
                        ..
                    } = self.replay.buffer.pop().expect("valid action event");
                    self.handle_action(s, slot, action, pressed, repeat)?;
                }
                Ordering::Less => {
                    log::warn!(
                        "Encountered action event out of order: {} < {}",
                        action_event.frame,
                        current_frame
                    );
                    self.replay.buffer.pop();
                }
                Ordering::Greater => break,
            }
        }
        if self.replay.buffer.is_empty() {
            self.stop_replay();
        }
        Ok(())
    }
}

impl Nes {
    #[inline]
    const fn action_event(
        &self,
        slot: Slot,
        action: Action,
        pressed: bool,
        repeat: bool,
    ) -> ActionEvent {
        ActionEvent {
            frame: self.control_deck.frame_number(),
            slot,
            action,
            pressed,
            repeat,
        }
    }

    #[inline]
    fn get_controller_slot(&self, controller_id: ControllerId) -> Option<Slot> {
        self.players.iter().find_map(|(&slot, &id)| {
            if id == controller_id {
                Some(slot)
            } else {
                None
            }
        })
    }

    fn handle_nes_state(&mut self, s: &mut PixState, state: NesState) -> NesResult<bool> {
        if self.replay.mode == ReplayMode::Recording {
            return Ok(false);
        }
        match state {
            NesState::Quit => {
                self.pause_play();
                s.quit();
            }
            NesState::TogglePause => self.toggle_pause(s)?,
            NesState::SoftReset => {
                self.error = None;
                self.control_deck.reset(Kind::Soft);
                self.add_message("Reset");
                if self.debugger.is_some() && self.mode != Mode::Paused {
                    self.mode = Mode::Paused;
                }
            }
            NesState::HardReset => {
                self.error = None;
                self.control_deck.reset(Kind::Hard);
                self.add_message("Power Cycled");
                if self.debugger.is_some() {
                    self.mode = Mode::Paused;
                }
            }
            NesState::MapperRevision(_) => todo!("mapper revision"),
        }
        Ok(true)
    }

    fn handle_feature(&mut self, s: &mut PixState, feature: Feature, pressed: bool, repeat: bool) {
        if feature == Feature::Rewind {
            if repeat {
                if self.config.rewind {
                    self.mode = Mode::Rewinding;
                } else {
                    self.add_message("Rewind disabled. You can enable it in the Config menu.");
                }
            } else if !pressed {
                if self.mode == Mode::Rewinding {
                    self.resume_play();
                } else {
                    self.instant_rewind();
                }
            }
        } else if pressed {
            match feature {
                Feature::ToggleGameplayRecording => match self.replay.mode {
                    ReplayMode::Off => self.start_replay(),
                    ReplayMode::Recording | ReplayMode::Playback => self.stop_replay(),
                },
                Feature::ToggleSoundRecording => self.toggle_sound_recording(s),
                Feature::TakeScreenshot => self.save_screenshot(s),
                Feature::SaveState => self.save_state(self.config.save_slot),
                Feature::LoadState => self.load_state(self.config.save_slot),
                Feature::Rewind => (), // Handled above
            }
        }
    }

    fn handle_setting(
        &mut self,
        s: &mut PixState,
        setting: Setting,
        pressed: bool,
        repeat: bool,
    ) -> NesResult<bool> {
        if setting == Setting::FastForward {
            if repeat {
                self.set_speed(2.0);
            } else if !pressed {
                self.set_speed(1.0);
            }
            Ok(true)
        } else if pressed {
            match setting {
                Setting::SetSaveSlot(slot) => {
                    self.config.save_slot = slot;
                    self.add_message(&format!("Set Save Slot to {slot}"));
                }
                Setting::ToggleFullscreen => {
                    self.config.fullscreen = !self.config.fullscreen;
                    s.fullscreen(self.config.fullscreen)?;
                }
                Setting::ToggleVsync => {
                    self.config.vsync = !self.config.vsync;
                    s.vsync(self.config.vsync)?;
                    if self.config.vsync {
                        self.add_message("Vsync Enabled");
                    } else {
                        self.add_message("Vsync Disabled");
                    }
                }
                Setting::ToggleNtscFilter => {
                    self.config.filter = match self.config.filter {
                        VideoFilter::Pixellate => VideoFilter::Ntsc,
                        VideoFilter::Ntsc => VideoFilter::Pixellate,
                    };
                    self.control_deck.set_filter(self.config.filter);
                }
                Setting::ToggleSound => {
                    self.config.sound = !self.config.sound;
                    if self.config.sound {
                        self.add_message("Sound Enabled");
                    } else {
                        self.add_message("Sound Disabled");
                    }
                }
                Setting::TogglePulse1 => self.control_deck.toggle_channel(Channel::Pulse1),
                Setting::TogglePulse2 => self.control_deck.toggle_channel(Channel::Pulse2),
                Setting::ToggleTriangle => self.control_deck.toggle_channel(Channel::Triangle),
                Setting::ToggleNoise => self.control_deck.toggle_channel(Channel::Noise),
                Setting::ToggleDmc => self.control_deck.toggle_channel(Channel::Dmc),
                Setting::IncSpeed => self.change_speed(0.25),
                Setting::DecSpeed => self.change_speed(-0.25),
                // Toggling fast forward happens on key release
                _ => return Ok(false),
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn handle_joypad_pressed(&mut self, slot: Slot, button: JoypadBtn, pressed: bool) -> bool {
        if self.mode != Mode::Playing {
            return false;
        }
        let joypad = self.control_deck.joypad_mut(slot);
        if !self.config.concurrent_dpad && pressed {
            match button {
                JoypadBtn::Left => joypad.set_button(JoypadBtnState::RIGHT, false),
                JoypadBtn::Right => joypad.set_button(JoypadBtnState::LEFT, false),
                JoypadBtn::Up => joypad.set_button(JoypadBtnState::DOWN, false),
                JoypadBtn::Down => joypad.set_button(JoypadBtnState::UP, false),
                _ => (),
            }
        }
        joypad.set_button(button.into(), pressed);

        // Ensure that primary button isn't stuck pressed
        match button {
            JoypadBtn::TurboA => joypad.set_button(JoypadBtnState::A, pressed),
            JoypadBtn::TurboB => joypad.set_button(JoypadBtnState::B, pressed),
            _ => (),
        };
        true
    }

    fn handle_debug(
        &mut self,
        s: &mut PixState,
        action: DebugAction,
        repeat: bool,
    ) -> NesResult<bool> {
        let debugging = self.debugger.is_some();
        match action {
            DebugAction::ToggleCpuDebugger if !repeat => self.toggle_debugger(s)?,
            DebugAction::TogglePpuDebugger if !repeat => self.toggle_ppu_viewer(s)?,
            DebugAction::ToggleApuDebugger if !repeat => self.toggle_apu_viewer(s)?,
            DebugAction::StepInto if debugging => self.debug_step_into(s)?,
            DebugAction::StepOver if debugging => self.debug_step_over(s)?,
            DebugAction::StepOut if debugging => self.debug_step_out(s)?,
            DebugAction::StepFrame if debugging => self.debug_step_frame(s)?,
            DebugAction::StepScanline if debugging => self.debug_step_scanline(s)?,
            DebugAction::IncScanline => {
                if let Some(ref mut viewer) = self.ppu_viewer {
                    let increment = if s.keymod_down(KeyMod::SHIFT) { 10 } else { 1 };
                    viewer.inc_scanline(increment);
                }
            }
            DebugAction::DecScanline => {
                if let Some(ref mut viewer) = self.ppu_viewer {
                    let decrement = if s.keymod_down(KeyMod::SHIFT) { 10 } else { 1 };
                    viewer.dec_scanline(decrement);
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn debug_step_into(&mut self, s: &mut PixState) -> NesResult<()> {
        self.pause_play();
        if let Err(err) = self.control_deck.clock_instr() {
            self.handle_emulation_error(s, &err)?;
        }
        Ok(())
    }

    fn next_instr(&mut self) -> Instr {
        let pc = self.control_deck.cpu().pc();
        let opcode = self.control_deck.cpu().peek(pc, Access::Dummy);
        Cpu::INSTRUCTIONS[opcode as usize]
    }

    fn debug_step_over(&mut self, s: &mut PixState) -> NesResult<()> {
        self.pause_play();
        let instr = self.next_instr();
        if let Err(err) = self.control_deck.clock_instr() {
            self.handle_emulation_error(s, &err)?;
        }
        if instr.op() == Operation::JSR {
            let rti_addr = self.control_deck.cpu().peek_stack_u16().wrapping_add(1);
            while self.control_deck.cpu().pc() != rti_addr {
                if let Err(err) = self.control_deck.clock_instr() {
                    self.handle_emulation_error(s, &err)?;
                    break;
                }
            }
        }
        Ok(())
    }

    fn debug_step_out(&mut self, s: &mut PixState) -> NesResult<()> {
        let mut instr = self.next_instr();
        while !matches!(instr.op(), Operation::RTS | Operation::RTI) {
            if let Err(err) = self.control_deck.clock_instr() {
                self.handle_emulation_error(s, &err)?;
                break;
            }
            instr = self.next_instr();
        }
        if let Err(err) = self.control_deck.clock_instr() {
            self.handle_emulation_error(s, &err)?;
        }

        Ok(())
    }

    fn debug_step_frame(&mut self, s: &mut PixState) -> NesResult<()> {
        self.pause_play();
        if let Err(err) = self.control_deck.clock_frame() {
            self.handle_emulation_error(s, &err)?;
        }
        Ok(())
    }

    fn debug_step_scanline(&mut self, s: &mut PixState) -> NesResult<()> {
        self.pause_play();
        if let Err(err) = self.control_deck.clock_scanline() {
            self.handle_emulation_error(s, &err)?;
        }
        Ok(())
    }
}
