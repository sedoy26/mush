use std::{
    io::{self, Stdout, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, queue, style,
    terminal::{self, ClearType},
};
#[cfg(unix)]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use mush_core::state::{
    audio::AudioDeviceSelection,
    drums::{get_bank, DrumVoice, NUM_DRUM_BANKS},
    midi::{MidiBindingTarget, MidiChannel, MidiLearnMode},
    project::ProjectTarget,
    synth::Waveform,
    ui::{SettingsPage, TabFocus, Theme, VisualFx, VisualMode},
    AppState, MAX_VOICES, NUM_PATTERNS, NUM_STEPS,
};
use mush_core::visuals::{Framebuffer, ParamValue, VisualRegistry};
use mush_core::Runtime;

const INPUT_POLL_MS: u64 = 16;

fn main() -> Result<()> {
    let base_dir = detect_base_dir()?;
    let mut runtime = Runtime::new(base_dir)?;
    runtime.start_audio()?;

    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        cursor::Hide
    )?;
    // Kitty-style keyboard protocol: reliable Press/Repeat/Release kinds for plain keys (Unix).
    // Without this, many terminals never emit Release, so `keyboard_note` sticks and chromatic
    // keys stop firing sample/synth note-ons after the first press.
    #[cfg(unix)]
    {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        );
        stdout.flush()?;
    }

    let mut ui = UiLocalState::default();
    let result = run_app(&mut stdout, &mut runtime, &mut ui);

    #[cfg(unix)]
    {
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
        let _ = stdout.flush();
    }
    terminal::disable_raw_mode()?;
    execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)?;
    result
}

fn detect_base_dir() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    if cwd.join("mu.sh").exists() {
        return Ok(cwd);
    }
    if cwd.file_name().and_then(|v| v.to_str()) == Some("rust") {
        if let Some(parent) = cwd.parent() {
            if parent.join("mu.sh").exists() {
                return Ok(parent.to_path_buf());
            }
        }
    }
    Ok(cwd)
}

fn run_app(stdout: &mut Stdout, runtime: &mut Runtime, ui: &mut UiLocalState) -> Result<()> {
    let (term_w, term_h) = terminal::size().unwrap_or((120, 40));
    let mut last_size = (term_w, term_h);
    let mut last_frame = render(runtime, ui)?;
    draw_frame_full(stdout, &last_frame)?;
    draw_visual_overlay(stdout, ui)?;
    draw_overlay_mask_region(stdout, &last_frame, ui)?;
    stdout.flush()?;
    loop {
        runtime.poll_background()?;
        runtime.sync_audio();  // Sync UI state to audio and read reactive levels
        update_keyboard_note_timeout(runtime, ui);
        
        let (cur_w, cur_h) = terminal::size().unwrap_or((120, 40));
        let size_changed = (cur_w, cur_h) != last_size;
        
        let frame = render(runtime, ui)?;
        let theme_changed = frame.theme != last_frame.theme;
        
        if size_changed || theme_changed {
            // Full redraw on resize or theme change
            draw_frame_full(stdout, &frame)?;
            last_size = (cur_w, cur_h);
        } else {
            // Differential update
            draw_frame_diff(stdout, &last_frame, &frame)?;
        }
        // Overlay framebuffer visuals with their native colors
        draw_visual_overlay(stdout, ui)?;
        // Redraw settings/help panel on top of visual overlay
        draw_overlay_mask_region(stdout, &frame, ui)?;
        // Single flush per frame - consolidated here
        stdout.flush()?;
        last_frame = frame;

        if event::poll(Duration::from_millis(INPUT_POLL_MS))? {
            if let Event::Key(key) = event::read()? {
                if handle_key(runtime, ui, key)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn draw_frame_full(stdout: &mut Stdout, frame: &RenderedFrame) -> Result<()> {
    queue!(stdout, terminal::Clear(ClearType::All))?;
    let mut cur_style: Option<UiStyle> = None;
    
    for (row_idx, row) in frame.cells.iter().enumerate() {
        queue!(stdout, cursor::MoveTo(0, row_idx as u16))?;
        
        for cell in row {
            // Always output style if it differs from current
            if cur_style != Some(cell.style) {
                queue!(stdout, style::Print(style_seq(frame.theme, cell.style)))?;
                cur_style = Some(cell.style);
            }
            queue!(stdout, style::Print(cell.ch))?;
        }
    }
    queue!(stdout, style::Print("\x1b[0m"))?;
    // No flush here - consolidated to end of frame
    Ok(())
}

fn draw_frame_diff(stdout: &mut Stdout, prev: &RenderedFrame, curr: &RenderedFrame) -> Result<()> {
    let mut cur_style: Option<UiStyle> = None;
    
    // Batch consecutive changed cells into runs for efficiency
    let mut run_start: Option<(u16, u16)> = None;
    let mut run_chars = String::new();
    let mut run_style = UiStyle::Plain;
    
    let flush_run = |stdout: &mut Stdout, start: Option<(u16, u16)>, chars: &mut String, style: UiStyle, cur_style: &mut Option<UiStyle>, theme: Theme| -> Result<()> {
        if let Some((x, y)) = start {
            if !chars.is_empty() {
                queue!(stdout, cursor::MoveTo(x, y))?;
                if *cur_style != Some(style) {
                    *cur_style = Some(style);
                    queue!(stdout, style::Print(style_seq(theme, style)))?;
                }
                queue!(stdout, style::Print(chars.as_str()))?;
                chars.clear();
            }
        }
        Ok(())
    };
    
    for (row_idx, (prev_row, curr_row)) in prev.cells.iter().zip(curr.cells.iter()).enumerate() {
        for (col_idx, (prev_cell, curr_cell)) in prev_row.iter().zip(curr_row.iter()).enumerate() {
            if prev_cell != curr_cell {
                // Check if we can extend the current run
                if let Some((rx, ry)) = run_start {
                    let expected_x = rx as usize + run_chars.len();
                    if row_idx == ry as usize && col_idx == expected_x && curr_cell.style == run_style {
                        // Extend run
                        run_chars.push(curr_cell.ch);
                        continue;
                    } else {
                        // Flush previous run and start new one
                        flush_run(stdout, run_start, &mut run_chars, run_style, &mut cur_style, curr.theme)?;
                    }
                }
                // Start new run
                run_start = Some((col_idx as u16, row_idx as u16));
                run_style = curr_cell.style;
                run_chars.push(curr_cell.ch);
            } else if run_start.is_some() {
                // Cell unchanged, flush any pending run
                flush_run(stdout, run_start, &mut run_chars, run_style, &mut cur_style, curr.theme)?;
                run_start = None;
            }
        }
        // End of row - flush run
        if run_start.is_some() {
            flush_run(stdout, run_start, &mut run_chars, run_style, &mut cur_style, curr.theme)?;
            run_start = None;
        }
    }
    
    // Handle size differences (new rows in current frame)
    for row_idx in prev.cells.len()..curr.cells.len() {
        queue!(stdout, cursor::MoveTo(0, row_idx as u16))?;
        for cell in &curr.cells[row_idx] {
            if cur_style != Some(cell.style) {
                cur_style = Some(cell.style);
                queue!(stdout, style::Print(style_seq(curr.theme, cell.style)))?;
            }
            queue!(stdout, style::Print(cell.ch))?;
        }
    }
    
    queue!(stdout, style::Print("\x1b[0m"))?;
    // No flush here - consolidated to end of frame
    Ok(())
}

/// Render framebuffer-based visual lines with embedded ANSI colors directly to terminal.
/// This overlays the colored visual on top of the canvas placeholder.
/// Clips around the settings/help mask region (only the exact rectangle, not full rows).
fn draw_visual_overlay(stdout: &mut Stdout, ui: &UiLocalState) -> Result<()> {
    use std::io::Write;
    
    // Skip if no colored overlay to draw (Camera/Scope modes don't use this)
    if ui.visual_colored_lines.is_empty() {
        return Ok(());
    }
    
    let fb_height = ui.visual_fb.height() as usize;
    let fb_width = ui.visual_fb.width() as usize;
    if fb_height == 0 || fb_width == 0 {
        return Ok(());
    }
    
    let (vx, vy) = ui.visual_pos;
    let mask = ui.overlay_mask; // (mx, my, mw, mh)
    
    for row_idx in 0..fb_height {
        let line_y = vy + row_idx;
        
        // Check if this row overlaps with mask vertically
        let row_overlaps = if let Some((_, my, _, mh)) = mask {
            line_y >= my && line_y < my + mh
        } else {
            false
        };
        
        if !row_overlaps {
            // Fast path: no overlap, use pre-rendered colored line
            if row_idx < ui.visual_colored_lines.len() {
                queue!(stdout, cursor::MoveTo(vx as u16, line_y as u16))?;
                stdout.write_all(ui.visual_colored_lines[row_idx].as_bytes())?;
            }
        } else {
            // Slow path: render cell by cell, skipping mask region
            let (mx, _, mw, _) = mask.unwrap();
            let mask_start = mx;
            let mask_end = mx + mw;
            
            let row = ui.visual_fb.row(row_idx as u16);
            let mut last_color: Option<(u8, u8, u8)> = None;
            let mut need_move = true;
            
            for (col, cell) in row.iter().enumerate() {
                let abs_x = vx + col;
                
                // Skip cells inside the mask
                if abs_x >= mask_start && abs_x < mask_end {
                    need_move = true;
                    continue;
                }
                
                if need_move {
                    queue!(stdout, cursor::MoveTo(abs_x as u16, line_y as u16))?;
                    need_move = false;
                }
                
                // Output color escape if changed
                let cur = (cell.fg.r, cell.fg.g, cell.fg.b);
                if last_color != Some(cur) {
                    let esc = format!("\x1b[38;2;{};{};{}m", cur.0, cur.1, cur.2);
                    stdout.write_all(esc.as_bytes())?;
                    last_color = Some(cur);
                }
                
                // Output character
                let mut buf = [0u8; 4];
                let s = cell.ch.encode_utf8(&mut buf);
                stdout.write_all(s.as_bytes())?;
            }
        }
    }
    
    stdout.write_all(b"\x1b[0m")?;
    // No flush here - consolidated to end of frame
    Ok(())
}

/// Redraw the settings/help panel region on top of visual overlay
fn draw_overlay_mask_region(stdout: &mut Stdout, frame: &RenderedFrame, ui: &UiLocalState) -> Result<()> {
    let Some((mx, my, mw, mh)) = ui.overlay_mask else {
        return Ok(());
    };
    
    let mut cur_style: Option<UiStyle> = None;
    
    for row_idx in my..(my + mh).min(frame.cells.len()) {
        if row_idx >= frame.cells.len() {
            continue;
        }
        let row = &frame.cells[row_idx];
        queue!(stdout, cursor::MoveTo(mx as u16, row_idx as u16))?;
        
        for col_idx in mx..(mx + mw).min(row.len()) {
            if col_idx >= row.len() {
                continue;
            }
            let cell = &row[col_idx];
            if cur_style != Some(cell.style) {
                queue!(stdout, style::Print(style_seq(frame.theme, cell.style)))?;
                cur_style = Some(cell.style);
            }
            queue!(stdout, style::Print(cell.ch))?;
        }
    }
    
    queue!(stdout, style::Print("\x1b[0m"))?;
    // No flush here - consolidated to end of frame
    Ok(())
}

fn handle_key(runtime: &mut Runtime, ui: &mut UiLocalState, key: KeyEvent) -> Result<bool> {
    // Key-up uses `KeyEventKind::Release` (enable Kitty keyboard flags in `main` for plain letters).
    match key.kind {
        KeyEventKind::Release => {
            let mut state = runtime.state.lock();
            if let Some(offset) = key_to_offset(&key.code) {
                let matched = Some(offset) == ui.keyboard_note;
                if matched {
                    ui.keyboard_note = None;
                    ui.keyboard_note_started_at = None;
                    ui.keyboard_note_last_repeat_at = None;
                    ui.keyboard_note_repeat_count = 0;
                }
                match state.ui.tab_focus {
                    TabFocus::Sample => {
                        let note = state.sample.note_for_keyboard_offset(offset);
                        let sample_ok = state.sample.play_enabled && state.sample.has_audio();
                        drop(state);
                        if sample_ok {
                            runtime.queue_sample_note_off(note);
                        }
                    }
                    TabFocus::Synth | TabFocus::Drums => {
                        if matched {
                            state.synth.key_offset = None;
                            state.synth.key_note_on = false;
                        }
                    }
                }
            }
            return Ok(false);
        }
        KeyEventKind::Press | KeyEventKind::Repeat => {}
    }

    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(true);
    }

    // Global mix WAV: Shift+g or uppercase G — must run before tab handlers so `g` chromatic on synth/sample never swallows it.
    if key_matches_shifted_base_letter(&key, 'g') {
        let mut state = runtime.state.lock();
        state.audio.global_recording.recording = !state.audio.global_recording.recording;
        return Ok(false);
    }

    if runtime.state.lock().ui.settings_open {
        return handle_settings_key(runtime, ui, key).map(|_| false);
    }

    let mut state = runtime.state.lock();
    if key_matches_shifted_base_letter(&key, 'h') {
        let new_state = !state.ui.help_open;
        state.ui.help_open = new_state;
        if new_state {
            state.ui.settings_open = false;
            ui.help_scroll = 0;
        } else {
            ui.help_scroll = 0;
        }
        return Ok(false);
    }
    if key_matches_shifted_base_letter(&key, 's') {
        let new_state = !state.ui.settings_open;
        state.ui.settings_open = new_state;
        if new_state {
            state.ui.help_open = false;
            ui.help_scroll = 0;
            state.project.available =
                mush_core::project_io::list_projects(runtime.base_dir()).unwrap_or_default();
            if state.project.available.is_empty() {
                ui.project_index = 0;
            } else {
                ui.project_index =
                    ui.project_index
                        .min(state.project.available.len().saturating_sub(1));
            }
        }
        return Ok(false);
    }
    if state.ui.help_open {
        match key.code {
            KeyCode::Esc => {
                state.ui.help_open = false;
                ui.help_scroll = 0;
                return Ok(false);
            }
            KeyCode::Up => {
                ui.help_scroll = ui.help_scroll.saturating_sub(1);
                return Ok(false);
            }
            KeyCode::Down => {
                ui.help_scroll = ui.help_scroll.saturating_add(1);
                return Ok(false);
            }
            KeyCode::Char('q') => return Ok(true),
            _ => return Ok(false),
        }
    }
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Tab => {
            state.ui.tab_focus = match state.ui.tab_focus {
                TabFocus::Synth => TabFocus::Drums,
                TabFocus::Drums => TabFocus::Sample,
                TabFocus::Sample => TabFocus::Synth,
            };
        }
        _ => {
            match state.ui.tab_focus {
                TabFocus::Synth => {
                    drop(state);
                    handle_synth_key(runtime, ui, key)?;
                }
                TabFocus::Sample => {
                    drop(state);
                    handle_sample_key(runtime, ui, key)?;
                }
                TabFocus::Drums => {
                    handle_drum_key(ui, key, &mut state);
                }
            }
        }
    }

    Ok(false)
}

fn handle_settings_key(runtime: &mut Runtime, ui: &mut UiLocalState, key: KeyEvent) -> Result<()> {
    let mut state = runtime.state.lock();
    if key.code == KeyCode::Esc || key_matches_shifted_base_letter(&key, 's') {
        state.ui.settings_open = false;
        return Ok(());
    }
    match key.code {
        KeyCode::Char('[') => {
            state.ui.settings_page = prev_page(state.ui.settings_page);
            ui.settings_cursor = 0;
        }
        KeyCode::Char(']') => {
            state.ui.settings_page = next_page(state.ui.settings_page);
            ui.settings_cursor = 0;
        }
        KeyCode::Up => ui.settings_cursor = ui.settings_cursor.saturating_sub(1),
        KeyCode::Down => {
            ui.settings_cursor = (ui.settings_cursor + 1)
                .min(settings_row_count(state.ui.settings_page, state.ui.visual_mode).saturating_sub(1));
        }
        KeyCode::Left => {
            drop(state);
            adjust_setting(runtime, ui, -1)?;
        }
        KeyCode::Right => {
            drop(state);
            adjust_setting(runtime, ui, 1)?;
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            drop(state);
            activate_setting(runtime, ui)?;
        }
        _ => {}
    }
    Ok(())
}

/// Arrows: **pitch** on ←→ (synth `base_midi` / sample **transpose** `pitch_semitones` / drum step) and **volume** on ↑↓
/// (synth level / sample gain). Sample **root** (anchor key) is S→MAIN **Smp root**. Drums: **↑↓** voice; **[** **]** vol.
fn shared_navigation_keys(
    state: &mut AppState,
    ui: &mut UiLocalState,
    tab: TabFocus,
    key: &KeyEvent,
) -> bool {
    match tab {
        TabFocus::Synth => match key.code {
            KeyCode::Left => {
                state.synth.base_midi = (state.synth.base_midi - 1).clamp(0, 127);
                true
            }
            KeyCode::Right => {
                state.synth.base_midi = (state.synth.base_midi + 1).clamp(0, 127);
                true
            }
            KeyCode::Up => {
                state.synth.volume = (state.synth.volume + 0.05).clamp(0.0, 1.0);
                true
            }
            KeyCode::Down => {
                state.synth.volume = (state.synth.volume - 0.05).clamp(0.0, 1.0);
                true
            }
            _ => false,
        },
        TabFocus::Sample => match key.code {
            KeyCode::Left => {
                state.sample.pitch_semitones = (state.sample.pitch_semitones - 0.5).clamp(-24.0, 24.0);
                true
            }
            KeyCode::Right => {
                state.sample.pitch_semitones = (state.sample.pitch_semitones + 0.5).clamp(-24.0, 24.0);
                true
            }
            KeyCode::Up => {
                state.sample.gain = (state.sample.gain + 0.05).clamp(0.0, 2.5);
                true
            }
            KeyCode::Down => {
                state.sample.gain = (state.sample.gain - 0.05).clamp(0.0, 2.5);
                true
            }
            _ => false,
        },
        TabFocus::Drums => match key.code {
            KeyCode::Left => {
                ui.drum_step = ui.drum_step.saturating_sub(1);
                true
            }
            KeyCode::Right => {
                ui.drum_step = (ui.drum_step + 1).min(31);
                true
            }
            KeyCode::Up => {
                ui.drum_voice = ui.drum_voice.saturating_sub(1);
                true
            }
            KeyCode::Down => {
                ui.drum_voice = (ui.drum_voice + 1).min(5);
                true
            }
            KeyCode::Char('[') => {
                state.drums.volumes[ui.drum_voice] =
                    (state.drums.volumes[ui.drum_voice] - 0.05).clamp(0.0, 1.0);
                true
            }
            KeyCode::Char(']') => {
                state.drums.volumes[ui.drum_voice] =
                    (state.drums.volumes[ui.drum_voice] + 0.05).clamp(0.0, 1.0);
                true
            }
            _ => false,
        },
    }
}

fn handle_synth_key(runtime: &mut Runtime, ui: &mut UiLocalState, key: KeyEvent) -> Result<()> {
    // T/Y/U are on the chromatic row but Shift+T/Y/U are loop controls (Kitty sends `t`+Shift, etc.).
    let loop_combo_ty_u = key_matches_shifted_base_letter(&key, 't')
        || key_matches_shifted_base_letter(&key, 'y')
        || key_matches_shifted_base_letter(&key, 'u');

    if !loop_combo_ty_u {
        if let Some(offset) = key_to_offset(&key.code) {
            // Only swallow key-repeat bursts for the *same* held key. Do not treat a fresh `Press`
            // as "already held" — many terminals omit `Release`, so `keyboard_note` can stay set.
            let is_same_key_autorepeat =
                matches!(key.kind, KeyEventKind::Repeat) && ui.keyboard_note == Some(offset);
            if is_same_key_autorepeat {
                ui.keyboard_note_repeat_count += 1;
                ui.keyboard_note_last_repeat_at = Some(Instant::now());
                return Ok(());
            }
            ui.keyboard_note = Some(offset);
            ui.keyboard_note_repeat_count = 0;
            ui.keyboard_note_started_at = Some(Instant::now());
            ui.keyboard_note_last_repeat_at = Some(Instant::now());
            let mut state = runtime.state.lock();
            state.synth.key_offset = Some(offset);
            state.synth.key_note_on = true;
            return Ok(());
        }
    }

    let mut state = runtime.state.lock();
    if shared_navigation_keys(&mut state, ui, TabFocus::Synth, &key) {
        return Ok(());
    }
    match key.code {
        KeyCode::Char(' ') => {
            state.synth.key_note_on = false;
            state.synth.key_offset = None;
            ui.keyboard_note = None;
            ui.keyboard_note_repeat_count = 0;
            ui.keyboard_note_started_at = None;
            ui.keyboard_note_last_repeat_at = None;
            ui.keyboard_note_released_at = Some(Instant::now());
            return Ok(());
        }
        KeyCode::Char('1') => state.synth.active_osc = 0,
        KeyCode::Char('2') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.synth.active_osc = 1;
        }
        KeyCode::Char('z') => cycle_waveform(&mut state, -1),
        KeyCode::Char('x') => cycle_waveform(&mut state, 1),
        // Attack: plain [ ] (terminals that send Shift+[ as Char('{') use the arms below instead).
        KeyCode::Char('[') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.synth.attack = (state.synth.attack - 0.005).clamp(0.001, 2.0)
        }
        KeyCode::Char(']') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.synth.attack = (state.synth.attack + 0.005).clamp(0.001, 2.0)
        }
        // Some terminals (Kitty) report Shift+[ as Char('[')+Shift, not Char('{') — treat as release.
        KeyCode::Char('[') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.synth.release = (state.synth.release - 0.005).clamp(0.0001, 4.0)
        }
        KeyCode::Char(']') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.synth.release = (state.synth.release + 0.005).clamp(0.0001, 4.0)
        }
        KeyCode::Char('{') => state.synth.release = (state.synth.release - 0.005).clamp(0.0001, 4.0),
        KeyCode::Char('}') => state.synth.release = (state.synth.release + 0.005).clamp(0.0001, 4.0),
        KeyCode::Char('m') => {
            state.synth.gate_mode = if matches!(
                state.synth.gate_mode,
                mush_core::state::synth::GateMode::Trigger
            ) {
                mush_core::state::synth::GateMode::Hold
            } else {
                mush_core::state::synth::GateMode::Trigger
            }
        }
        KeyCode::Char('p') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.synth.filter_on = !state.synth.filter_on
        }
        KeyCode::Char('-') => state.synth.cutoff = (state.synth.cutoff - 0.03).clamp(0.0, 1.0),
        KeyCode::Char('=') => state.synth.cutoff = (state.synth.cutoff + 0.03).clamp(0.0, 1.0),
        KeyCode::Char('_') => {
            state.synth.resonance = (state.synth.resonance - 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('+') => {
            state.synth.resonance = (state.synth.resonance + 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('l') => {
            state.synth.lfo_wave = match state.synth.lfo_wave {
                mush_core::state::synth::LfoWaveform::Sine => {
                    mush_core::state::synth::LfoWaveform::Triangle
                }
                mush_core::state::synth::LfoWaveform::Triangle => {
                    mush_core::state::synth::LfoWaveform::Square
                }
                mush_core::state::synth::LfoWaveform::Square => {
                    mush_core::state::synth::LfoWaveform::Sine
                }
            }
        }
        KeyCode::Char('o') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.synth.lfo_target = match state.synth.lfo_target {
                mush_core::state::synth::LfoTarget::Pitch => {
                    mush_core::state::synth::LfoTarget::Volume
                }
                mush_core::state::synth::LfoTarget::Volume => {
                    mush_core::state::synth::LfoTarget::Filter
                }
                mush_core::state::synth::LfoTarget::Filter => {
                    mush_core::state::synth::LfoTarget::Pitch
                }
            }
        }
        _ if key_matches_shifted_base_letter(&key, 'i') => {
            state.looper.play_gain = (state.looper.play_gain + 0.05).clamp(0.0, 1.0)
        }
        _ if key_matches_shifted_base_letter(&key, 'o') => {
            state.looper.play_gain = (state.looper.play_gain - 0.05).clamp(0.0, 1.0)
        }
        _ if key_shifted_digit_row(&key, '2', '@') => {
            state.looper.trim_start = (state.looper.trim_start + 0.02).clamp(0.0, 0.9)
        }
        _ if key_shifted_digit_row(&key, '3', '#') => {
            state.looper.trim_start = (state.looper.trim_start - 0.02).clamp(0.0, 0.9)
        }
        _ if key_shifted_digit_row(&key, '4', '$') => {
            state.looper.trim_end = (state.looper.trim_end + 0.02).clamp(0.0, 0.9)
        }
        _ if key_shifted_digit_row(&key, '5', '%') => {
            state.looper.trim_end = (state.looper.trim_end - 0.02).clamp(0.0, 0.9)
        }
        _ if key_shifted_digit_row(&key, '6', '^') => {
            state.looper.playback_speed = (state.looper.playback_speed * 1.1).clamp(0.25, 4.0)
        }
        _ if key_shifted_digit_row(&key, '7', '&') => {
            state.looper.playback_speed = (state.looper.playback_speed / 1.1).clamp(0.25, 4.0)
        }
        KeyCode::Char(',') => state.synth.lfo_rate = (state.synth.lfo_rate - 0.2).clamp(0.0, 20.0),
        KeyCode::Char('.') => state.synth.lfo_rate = (state.synth.lfo_rate + 0.2).clamp(0.0, 20.0),
        KeyCode::Char(';') => {
            state.synth.lfo_depth = (state.synth.lfo_depth - 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('"') | KeyCode::Char('\'') => {
            state.synth.lfo_depth = (state.synth.lfo_depth + 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('D') => state.synth.fx.drive = (state.synth.fx.drive - 0.03).clamp(0.0, 1.0),
        KeyCode::Char('F') => state.synth.fx.drive = (state.synth.fx.drive + 0.03).clamp(0.0, 1.0),
        KeyCode::Char('J') => {
            state.synth.fx.delay_mix = (state.synth.fx.delay_mix - 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('K') => {
            state.synth.fx.delay_mix = (state.synth.fx.delay_mix + 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('N') => {
            state.synth.fx.delay_feedback = (state.synth.fx.delay_feedback - 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('M') => {
            state.synth.fx.delay_feedback = (state.synth.fx.delay_feedback + 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('V') => {
            state.synth.fx.delay_time = (state.synth.fx.delay_time - 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('B') => {
            state.synth.fx.delay_time = (state.synth.fx.delay_time + 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('W') => {
            state.synth.fx.warmth = (state.synth.fx.warmth + 0.03).clamp(0.0, 1.0)
        }
        KeyCode::Char('A') => state.synth.fx.air = (state.synth.fx.air + 0.03).clamp(0.0, 1.0),
        KeyCode::Char('E') => {
            state.synth.fx.reverb = (state.synth.fx.reverb + 0.03).clamp(0.0, 1.0)
        }
        _ if key_matches_shifted_base_letter(&key, 'r') => {
            if state.looper.recording {
                state.looper.stop_recording();
            } else {
                state.looper.begin_replace();
            }
        }
        _ if key_matches_shifted_base_letter(&key, 't') => {
            if state.looper.overdub {
                state.looper.stop_recording();
            } else {
                state.looper.begin_overdub();
            }
        }
        _ if key_matches_shifted_base_letter(&key, 'y') => {
            state.looper.undo_last();
        }
        _ if key_matches_shifted_base_letter(&key, 'p') => {
            if state.looper.has_audio {
                state.looper.playing = !state.looper.playing;
            }
        }
        _ if key_matches_shifted_base_letter(&key, 'u') => {
            state.looper.clear();
        }
        // Drums chain: plain \ toggles list-play; | or Shift+\ appends current bank (terminals often send the latter as \\+Shift).
        KeyCode::Char('|') => {
            let p = state.drums.current_pattern;
            state.drums.chain_push(p);
        }
        KeyCode::Char('\\') => {
            let p = state.drums.current_pattern;
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                state.drums.chain_push(p);
            } else {
                state.drums.chain_mode = !state.drums.chain_mode;
            }
        }
        _ => {}
    }
    Ok(())
}

/// QWERTY chromatic + arrows for transpose/gain + **sample performance loop** only (no synth osc/FX/filter).
fn handle_sample_key(runtime: &mut Runtime, ui: &mut UiLocalState, key: KeyEvent) -> Result<()> {
    let loop_combo_ty_u = key_matches_shifted_base_letter(&key, 't')
        || key_matches_shifted_base_letter(&key, 'y')
        || key_matches_shifted_base_letter(&key, 'u');

    if !loop_combo_ty_u {
        if let Some(offset) = key_to_offset(&key.code) {
            let is_same_key_autorepeat =
                matches!(key.kind, KeyEventKind::Repeat) && ui.keyboard_note == Some(offset);
            if is_same_key_autorepeat {
                ui.keyboard_note_repeat_count += 1;
                ui.keyboard_note_last_repeat_at = Some(Instant::now());
                return Ok(());
            }
            // Same key still "down" (terminals often send extra `Press` while held). Do not retrigger;
            // the engine loops the sample until note-off / timeout. Do not bump `last_repeat_at` here
            // so idle timeout still sees quiet after the last real `Repeat` / initial press.
            if ui.keyboard_note == Some(offset) {
                return Ok(());
            }
            ui.keyboard_note = Some(offset);
            ui.keyboard_note_repeat_count = 0;
            ui.keyboard_note_started_at = Some(Instant::now());
            ui.keyboard_note_last_repeat_at = Some(Instant::now());
            let mut state = runtime.state.lock();
            if state.sample.has_audio() && !state.sample.play_enabled {
                state.sample.play_enabled = true;
            }
            let note = state.sample.note_for_keyboard_offset(offset);
            let sample_ok = state.sample.play_enabled && state.sample.has_audio();
            drop(state);
            if sample_ok {
                runtime.queue_sample_note_on(note, 127.0);
            }
            return Ok(());
        }
    }

    let mut state = runtime.state.lock();
    if shared_navigation_keys(&mut state, ui, TabFocus::Sample, &key) {
        return Ok(());
    }
    match key.code {
        KeyCode::Char(' ') => {
            if let Some(off) = ui.keyboard_note {
                let note = state.sample.note_for_keyboard_offset(off);
                let sample_ok = state.sample.play_enabled && state.sample.has_audio();
                ui.keyboard_note = None;
                ui.keyboard_note_repeat_count = 0;
                ui.keyboard_note_started_at = None;
                ui.keyboard_note_last_repeat_at = None;
                ui.keyboard_note_released_at = Some(Instant::now());
                drop(state);
                if sample_ok {
                    runtime.queue_sample_note_off(note);
                }
            } else {
                ui.keyboard_note_released_at = Some(Instant::now());
            }
            return Ok(());
        }
        _ if key_matches_shifted_base_letter(&key, 'i') => {
            state.sample.performance_loop.play_gain =
                (state.sample.performance_loop.play_gain + 0.05).clamp(0.0, 1.0);
        }
        _ if key_matches_shifted_base_letter(&key, 'o') => {
            state.sample.performance_loop.play_gain =
                (state.sample.performance_loop.play_gain - 0.05).clamp(0.0, 1.0);
        }
        _ if key_shifted_digit_row(&key, '2', '@') => {
            state.sample.performance_loop.trim_start =
                (state.sample.performance_loop.trim_start + 0.02).clamp(0.0, 0.9);
        }
        _ if key_shifted_digit_row(&key, '3', '#') => {
            state.sample.performance_loop.trim_start =
                (state.sample.performance_loop.trim_start - 0.02).clamp(0.0, 0.9);
        }
        _ if key_shifted_digit_row(&key, '4', '$') => {
            state.sample.performance_loop.trim_end =
                (state.sample.performance_loop.trim_end + 0.02).clamp(0.0, 0.9);
        }
        _ if key_shifted_digit_row(&key, '5', '%') => {
            state.sample.performance_loop.trim_end =
                (state.sample.performance_loop.trim_end - 0.02).clamp(0.0, 0.9);
        }
        _ if key_shifted_digit_row(&key, '6', '^') => {
            state.sample.performance_loop.playback_speed =
                (state.sample.performance_loop.playback_speed * 1.1).clamp(0.25, 4.0);
        }
        _ if key_shifted_digit_row(&key, '7', '&') => {
            state.sample.performance_loop.playback_speed =
                (state.sample.performance_loop.playback_speed / 1.1).clamp(0.25, 4.0);
        }
        _ if key_matches_shifted_base_letter(&key, 'r') => {
            let recording = state.sample.performance_loop.recording;
            drop(state);
            if recording {
                runtime.stop_sample_loop_recording();
            } else {
                runtime.start_sample_loop_recording();
            }
            return Ok(());
        }
        _ if key_matches_shifted_base_letter(&key, 't') => {
            let overdub = state.sample.performance_loop.overdub;
            drop(state);
            if overdub {
                runtime.stop_sample_loop_recording();
            } else {
                runtime.start_sample_loop_overdub();
            }
            return Ok(());
        }
        _ if key_matches_shifted_base_letter(&key, 'y') => {
            drop(state);
            runtime.undo_sample_loop();
            return Ok(());
        }
        _ if key_matches_shifted_base_letter(&key, 'p') => {
            drop(state);
            runtime.toggle_sample_loop_playback();
            return Ok(());
        }
        _ if key_matches_shifted_base_letter(&key, 'u') => {
            drop(state);
            runtime.clear_sample_loop();
            return Ok(());
        }
        _ => {}
    }
    Ok(())
}

fn update_keyboard_note_timeout(runtime: &mut Runtime, ui: &mut UiLocalState) {
    let Some(last_activity) = ui.keyboard_note_last_repeat_at else {
        return;
    };
    
    let Some(started_at) = ui.keyboard_note_started_at else {
        return;
    };
    
    // If we've seen repeat events for this key hold
    if ui.keyboard_note_repeat_count > 0 {
        let idle_ms = {
            let s = runtime.state.lock();
            if matches!(s.ui.tab_focus, TabFocus::Sample) {
                110u64
            } else {
                150u64
            }
        };
        if last_activity.elapsed() > Duration::from_millis(idle_ms) {
            let mut state = runtime.state.lock();
            let off = ui.keyboard_note.unwrap_or(0);
            match state.ui.tab_focus {
                TabFocus::Sample => {
                    let note = state.sample.note_for_keyboard_offset(off);
                    let sample_ok = state.sample.play_enabled && state.sample.has_audio();
                    ui.keyboard_note = None;
                    ui.keyboard_note_started_at = None;
                    ui.keyboard_note_last_repeat_at = None;
                    ui.keyboard_note_repeat_count = 0;
                    drop(state);
                    if sample_ok {
                        runtime.queue_sample_note_off(note);
                    }
                }
                TabFocus::Synth | TabFocus::Drums => {
                    state.synth.key_note_on = false;
                    state.synth.key_offset = None;
                    ui.keyboard_note = None;
                    ui.keyboard_note_started_at = None;
                    ui.keyboard_note_last_repeat_at = None;
                    ui.keyboard_note_repeat_count = 0;
                }
            }
        }
    } else {
        // No repeats yet — infer key-up after quiet (shorter in sample mode; synth keeps 600ms).
        let (hold_ms, gap_ms) = {
            let s = runtime.state.lock();
            if matches!(s.ui.tab_focus, TabFocus::Sample) {
                (320u64, 120u64)
            } else {
                (600u64, 150u64)
            }
        };
        if started_at.elapsed() > Duration::from_millis(hold_ms)
            && last_activity.elapsed() > Duration::from_millis(gap_ms)
        {
            let mut state = runtime.state.lock();
            let off = ui.keyboard_note.unwrap_or(0);
            match state.ui.tab_focus {
                TabFocus::Sample => {
                    let note = state.sample.note_for_keyboard_offset(off);
                    let sample_ok = state.sample.play_enabled && state.sample.has_audio();
                    ui.keyboard_note = None;
                    ui.keyboard_note_started_at = None;
                    ui.keyboard_note_last_repeat_at = None;
                    ui.keyboard_note_repeat_count = 0;
                    drop(state);
                    if sample_ok {
                        runtime.queue_sample_note_off(note);
                    }
                }
                TabFocus::Synth | TabFocus::Drums => {
                    state.synth.key_note_on = false;
                    state.synth.key_offset = None;
                    ui.keyboard_note = None;
                    ui.keyboard_note_started_at = None;
                    ui.keyboard_note_last_repeat_at = None;
                    ui.keyboard_note_repeat_count = 0;
                }
            }
        }
    }
}

fn handle_drum_key(ui: &mut UiLocalState, key: KeyEvent, state: &mut AppState) {
    let now = Instant::now();
    let clear_confirm = |ui: &mut UiLocalState| {
        ui.drum_clear_confirm = None;
    };
    let set_notice = |ui: &mut UiLocalState, message: String| {
        ui.drum_notice = Some(message);
        ui.drum_notice_until = Some(now + Duration::from_secs_f32(1.2));
    };
    let notice_active = |ui: &UiLocalState, confirm: DrumClearConfirm| {
        matches!(ui.drum_clear_confirm, Some(active) if active == confirm)
            && ui.drum_notice_until.map_or(false, |until| now < until)
    };

    if shared_navigation_keys(state, ui, TabFocus::Drums, &key) {
        clear_confirm(ui);
        return;
    }

    match key.code {
        KeyCode::Char(' ') => {
            clear_confirm(ui);
            let pattern = state.drums.current_pattern;
            let value = &mut state.drums.patterns[pattern][ui.drum_voice][ui.drum_step];
            *value = !*value;
        }
        KeyCode::Enter => {
            clear_confirm(ui);
            state.drums.running = !state.drums.running;
        }
        KeyCode::Char('r') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            clear_confirm(ui);
            state.drums.running = !state.drums.running;
        }
        KeyCode::Char('c') => {
            if notice_active(ui, DrumClearConfirm::Row(ui.drum_voice)) {
                state.drums.clear_voice(DrumVoice::ALL[ui.drum_voice]);
                set_notice(
                    ui,
                    format!(
                        "Cleared {} row",
                        ["kick", "snare", "clap", "hat", "tom", "cymb"][ui.drum_voice]
                    ),
                );
                ui.drum_clear_confirm = None;
            } else {
                ui.drum_clear_confirm = Some(DrumClearConfirm::Row(ui.drum_voice));
                set_notice(
                    ui,
                    format!(
                        "Press c again to clear {} row",
                        ["kick", "snare", "clap", "hat", "tom", "cymb"][ui.drum_voice]
                    ),
                );
            }
        }
        KeyCode::Char('X') => {
            if notice_active(ui, DrumClearConfirm::All) {
                state.drums.clear_all();
                set_notice(ui, "Cleared all drum steps".to_string());
                ui.drum_clear_confirm = None;
            } else {
                ui.drum_clear_confirm = Some(DrumClearConfirm::All);
                set_notice(
                    ui,
                    "Press Shift+X again to clear all drum steps".to_string(),
                );
            }
        }
        // Shift+1 / Shift+2 (or ! / @): load starter grids into the *current* pattern slot.
        _ if key_shifted_digit_row(&key, '1', '!') => {
            clear_confirm(ui);
            apply_pattern(state, 0);
            set_notice(ui, "Starter A loaded into this pattern".to_string());
        }
        _ if key_shifted_digit_row(&key, '2', '@') => {
            clear_confirm(ui);
            apply_pattern(state, 1);
            set_notice(ui, "Starter B loaded into this pattern".to_string());
        }
        // Plain 1-8: select which pattern bank is being edited (append banks to the list with | or Shift+\).
        KeyCode::Char(c @ '1'..='8') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            clear_confirm(ui);
            let idx = (c as u8 - b'1') as usize;
            if idx < NUM_PATTERNS {
                state.drums.current_pattern = idx;
                set_notice(ui, format!("Editing pattern {}", idx + 1));
            }
        }
        KeyCode::Char(',') => {
            clear_confirm(ui);
            state.drums.bpm = (state.drums.bpm - 1.0).clamp(40.0, 300.0)
        }
        KeyCode::Char('.') => {
            clear_confirm(ui);
            state.drums.bpm = (state.drums.bpm + 1.0).clamp(40.0, 300.0)
        }
        KeyCode::Char('<') => {
            clear_confirm(ui);
            state.drums.bpm = (state.drums.bpm - 5.0).clamp(40.0, 300.0)
        }
        KeyCode::Char('>') => {
            clear_confirm(ui);
            state.drums.bpm = (state.drums.bpm + 5.0).clamp(40.0, 300.0)
        }
        // Pattern navigation: { and } to cycle patterns
        KeyCode::Char('{') => {
            clear_confirm(ui);
            state.drums.current_pattern = state.drums.current_pattern.saturating_sub(1);
            set_notice(ui, format!("Editing pattern {}", state.drums.current_pattern + 1));
        }
        KeyCode::Char('}') => {
            clear_confirm(ui);
            state.drums.current_pattern = (state.drums.current_pattern + 1).min(NUM_PATTERNS - 1);
            set_notice(ui, format!("Editing pattern {}", state.drums.current_pattern + 1));
        }
        // Chain: plain \ toggles playing the bank list; | or Shift+\ appends current bank (many terminals emit Shift+\ as \\ + Shift).
        KeyCode::Char('|') => {
            clear_confirm(ui);
            state.drums.chain_push(state.drums.current_pattern);
            set_notice(ui, format!("Added bank {} to list (len={})", state.drums.current_pattern + 1, state.drums.chain.len()));
        }
        KeyCode::Char('\\') => {
            clear_confirm(ui);
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                state.drums.chain_push(state.drums.current_pattern);
                set_notice(ui, format!("Added bank {} to list (len={})", state.drums.current_pattern + 1, state.drums.chain.len()));
            } else {
                state.drums.chain_mode = !state.drums.chain_mode;
                set_notice(ui, format!("Play pattern list {}", if state.drums.chain_mode { "ON" } else { "OFF" }));
            }
        }
        // Backspace removes last pattern from chain
        KeyCode::Backspace => {
            clear_confirm(ui);
            if !state.drums.chain.is_empty() {
                state.drums.chain_pop();
                set_notice(ui, format!("Removed from chain (len={})", state.drums.chain.len()));
            }
        }
        // Delete clears chain
        KeyCode::Delete => {
            clear_confirm(ui);
            state.drums.chain_clear();
            set_notice(ui, "Chain cleared".to_string());
        }
        // Copy pattern: ( copies current pattern to prev, ) copies to next
        KeyCode::Char('(') => {
            clear_confirm(ui);
            if state.drums.current_pattern > 0 {
                let dest = state.drums.current_pattern - 1;
                state.drums.copy_pattern_to(dest);
                set_notice(ui, format!("Copied pattern {} to {}", state.drums.current_pattern + 1, dest + 1));
            }
        }
        KeyCode::Char(')') => {
            clear_confirm(ui);
            if state.drums.current_pattern < NUM_PATTERNS - 1 {
                let dest = state.drums.current_pattern + 1;
                state.drums.copy_pattern_to(dest);
                set_notice(ui, format!("Copied pattern {} to {}", state.drums.current_pattern + 1, dest + 1));
            }
        }
        _ => {}
    }
}

fn render(runtime: &mut Runtime, ui: &mut UiLocalState) -> Result<RenderedFrame> {
    let (term_w, term_h) = terminal::size().unwrap_or((120, 40));
    let term_w = term_w as usize;
    let term_h = term_h as usize;
    let state = runtime.state.lock().clone();
    let active_note = note_name(state.synth.current_play_midi());
    let _gate = if state.synth.note_active() {
        "ON"
    } else {
        "OFF"
    };
    let loop_state = if matches!(state.ui.tab_focus, TabFocus::Sample) {
        let lp = &state.sample.performance_loop;
        if lp.recording && !lp.overdub {
            "REC"
        } else if lp.overdub {
            "DUB"
        } else if lp.playing {
            "PLY"
        } else if lp.has_audio {
            "HLD"
        } else {
            "OFF"
        }
    } else if state.looper.recording && !state.looper.overdub {
        "REC"
    } else if state.looper.overdub {
        "DUB"
    } else if state.looper.playing {
        "PLY"
    } else if state.looper.has_audio {
        "HLD"
    } else {
        "OFF"
    };
    // Loop status text follows tab; gain/trim/speed row must match the same loop (synth vs sample).
    let loop_panel = if matches!(state.ui.tab_focus, TabFocus::Sample) {
        &state.sample.performance_loop
    } else {
        &state.looper
    };
    let mut canvas = Canvas::new(term_w, term_h.max(36));

    canvas.text_center_style(0, "♪ mush ♪", UiStyle::Header);
    canvas.text_style(
        term_w.saturating_sub(35),
        0,
        if state.ui.settings_open {
            "[SETTINGS S]"
        } else {
            " SETTINGS S "
        },
        if state.ui.settings_open {
            UiStyle::Scope
        } else {
            UiStyle::Value
        },
    );
    canvas.text_style(
        term_w.saturating_sub(21),
        0,
        if state.ui.help_open {
            "[HELP H]"
        } else {
            " HELP H "
        },
        if state.ui.help_open {
            UiStyle::Scope
        } else {
            UiStyle::Value
        },
    );
    canvas.text_style(
        term_w.saturating_sub(10),
        0,
        match state.ui.tab_focus {
            TabFocus::Synth => "[SYNTH]",
            TabFocus::Drums => "[DRUMS]",
            TabFocus::Sample => "[SAMPLE]",
        },
        UiStyle::Cursor,
    );

    // Draw outer window border
    let left_x = 2;
    let left_w = ((term_w.saturating_mul(22)) / 100).clamp(28, 36);
    let right_x = left_x + left_w + 1;
    let right_w = term_w.saturating_sub(right_x + 2);  // 2 char margin on right
    let visual_width = right_w.saturating_sub(2).max(24);  // 2 for box borders (1 each side)
    const FX_TOP: usize = 15;
    let fx_h: usize = 9;
    let sample_top: usize = FX_TOP + fx_h + 1;
    let sample_h: usize = 8;
    let song_top: usize = sample_top + sample_h + 1;
    let song_h: usize = 5;
    let song_bottom = song_top + song_h - 1;
    canvas.boxed_style(left_x, 2, left_w, 12, " OSC ", UiStyle::Scope);
    canvas.boxed_style(left_x, FX_TOP, left_w, fx_h, " FX ", UiStyle::Scope);
    canvas.boxed_style(left_x, sample_top, left_w, sample_h, " SAMPLE ", UiStyle::Scope);
    canvas.boxed_style(left_x, song_top, left_w, song_h, " SONG ", UiStyle::Scope);
    let drum_h = 10;
    let drum_top = term_h
        .saturating_sub(drum_h)
        .max(30)
        .max(song_bottom.saturating_add(2));
    let footer_y = drum_top.saturating_sub(1);
    let visual_box_h = drum_top.saturating_sub(3).max(6);
    canvas.boxed_style(
        right_x,
        2,
        right_w,
        visual_box_h,
        if matches!(state.ui.visual_mode, VisualMode::Camera) {
            " CAM "
        } else {
            " SCOPE "
        },
        UiStyle::Scope,
    );
    canvas.boxed_style(2, drum_top, term_w.saturating_sub(4), drum_h, " DRUMS ", UiStyle::Scope);

    // OSC section - row 3: oscillator selector
    canvas.text_style(left_x + 2, 3, "Edit", UiStyle::Label);
    canvas.text_style(
        left_x + 8,
        3,
        if state.synth.active_osc == 0 { "[1]" } else { " 1 " },
        if state.synth.active_osc == 0 { UiStyle::Active } else { UiStyle::Value },
    );
    canvas.text_style(
        left_x + 12,
        3,
        if state.synth.active_osc == 1 { "[2]" } else { " 2 " },
        if state.synth.active_osc == 1 { UiStyle::Active } else { UiStyle::Value },
    );
    canvas.text_style(left_x + 17, 3, "z/x wave", UiStyle::Hint);

    // OSC section - row 4: separator
    canvas.text_style(left_x + 2, 4, &"·".repeat(left_w.saturating_sub(4)), UiStyle::Hint);

    // OSC section - rows 5-6: oscillator details
    for (osc_idx, osc) in state.synth.oscillators.iter().enumerate() {
        let row = 5 + osc_idx;
        let oct_lbl = if osc.octave >= 0 {
            format!("{}x", 2_i32.pow(osc.octave as u32))
        } else {
            format!("1/{}x", 2_i32.pow((-osc.octave) as u32))
        };
        let wave_name = format!("{:?}", osc.waveform);
        canvas.text_style(
            left_x + 2,
            row,
            &format!("OSC{}", osc_idx + 1),
            if osc_idx == state.synth.active_osc { UiStyle::Active } else { UiStyle::Label },
        );
        canvas.text_style(
            left_x + 8,
            row,
            &short_label(
                &format!(
                    "{:<8} {:>3}%  {}  {:+.1}c",
                    wave_name,
                    (osc.level * 100.0) as i32,
                    oct_lbl,
                    osc.detune_cents
                ),
                left_w.saturating_sub(10),
            ),
            if osc_idx == state.synth.active_osc { UiStyle::Active } else { UiStyle::Value },
        );
    }

    // OSC section - row 7: separator
    canvas.text_style(left_x + 2, 7, &"·".repeat(left_w.saturating_sub(4)), UiStyle::Hint);

    // OSC section - row 8: volume
    canvas.text_style(left_x + 2, 8, "Vol", UiStyle::Label);
    canvas.text_style(
        left_x + 6,
        8,
        &format!(
            "{:>3}%  {}",
            (state.synth.volume * 100.0) as i32,
            bar(state.synth.volume, left_w.saturating_sub(16).clamp(6, 14))
        ),
        UiStyle::Bar,
    );

    // OSC section - row 9: mode and gate
    canvas.text_style(left_x + 2, 9, "Mode", UiStyle::Label);
    canvas.text_style(
        left_x + 7,
        9,
        if matches!(state.synth.gate_mode, mush_core::state::synth::GateMode::Hold) {
            "HOLD [m]"
        } else {
            "TRIG [m]"
        },
        UiStyle::Hint,
    );

    // OSC section - row 10: notes
    canvas.text_style(left_x + 2, 10, "Base", UiStyle::Label);
    canvas.text_style(left_x + 7, 10, &note_name(state.synth.base_midi), UiStyle::Value);
    canvas.text_style(left_x + 13, 10, "Play", UiStyle::Label);
    canvas.text_style(left_x + 18, 10, &active_note, UiStyle::Active);

    // OSC section - row 11: loop status
    canvas.text_style(left_x + 2, 11, "Loop", UiStyle::Label);
    canvas.text_style(
        left_x + 7,
        11,
        loop_state,
        if loop_state == "REC" || loop_state == "DUB" { UiStyle::Active } else { UiStyle::Value },
    );
    canvas.text_style(left_x + 13, 11, "Undo", UiStyle::Label);
    canvas.text_style(
        left_x + 18,
        11,
        &format!("{:02}", loop_panel.undo_stack.len()),
        UiStyle::Hint,
    );
    canvas.text_style(left_x + 22, 11, "XR", UiStyle::Label);
    canvas.text_style(
        left_x + 25,
        11,
        &format!("{:02}", state.audio.xruns),
        if state.audio.xruns > 0 { UiStyle::Filter } else { UiStyle::Hint },
    );

    // Loop section - row 12: separator
    canvas.text_style(left_x + 2, 12, &"·".repeat(left_w.saturating_sub(4)), UiStyle::Hint);

    // Loop section - row 13: loop controls
    canvas.text_style(left_x + 2, 13, "Lgn", UiStyle::Label);
    canvas.text_style(
        left_x + 6,
        13,
        &format!("{:>3}%", (loop_panel.play_gain * 100.0) as i32),
        UiStyle::Value,
    );
    
    canvas.text_style(left_x + 12, 13, "Srt", UiStyle::Label);
    canvas.text_style(
        left_x + 16,
        13,
        &format!("{:>3}%", (loop_panel.trim_start * 100.0) as i32),
        UiStyle::Value,
    );
    
    // End shows actual end position (100% - trim), so 0% trim = 100% end
    canvas.text_style(left_x + 22, 13, "End", UiStyle::Label);
    canvas.text_style(
        left_x + 26,
        13,
        &format!("{:>3}%", ((1.0 - loop_panel.trim_end) * 100.0) as i32),
        UiStyle::Value,
    );

    // Loop section - row 14: speed
    canvas.text_style(left_x + 2, 14, "Spd", UiStyle::Label);
    canvas.text_style(
        left_x + 6,
        14,
        &format!("{:.2}x", loop_panel.playback_speed),
        UiStyle::Value,
    );

    // FX section - row 16: Attack and Release
    canvas.text_style(left_x + 2, 16, "Atk", UiStyle::Label);
    canvas.text_style(left_x + 6, 16, &format!("{:.3}s", state.synth.attack), UiStyle::Value);
    canvas.text_style(left_x + 14, 16, "Rel", UiStyle::Label);
    canvas.text_style(left_x + 18, 16, &format!("{:.3}s", state.synth.release), UiStyle::Value);

    // FX section - row 17: ENV bar
    canvas.text_style(left_x + 2, 17, "ENV", UiStyle::Label);
    canvas.text_style(
        left_x + 6,
        17,
        &bar(state.synth.env, left_w.saturating_sub(10).clamp(8, 20)),
        UiStyle::Bar,
    );

    // FX section - row 18: LFO
    canvas.text_style(left_x + 2, 18, "LFO", UiStyle::Label);
    canvas.text_style(
        left_x + 6,
        18,
        &format!("{:?}", state.synth.lfo_wave),
        UiStyle::Value,
    );
    canvas.text_style(left_x + 14, 18, "→", UiStyle::Hint);
    canvas.text_style(
        left_x + 16,
        18,
        &format!("{:?}", state.synth.lfo_target),
        UiStyle::Value,
    );

    // FX section - row 19: LFO rate/depth + Filter
    canvas.text_style(left_x + 2, 19, "Rate", UiStyle::Label);
    canvas.text_style(left_x + 7, 19, &format!("{:.1}Hz", state.synth.lfo_rate), UiStyle::Value);
    canvas.text_style(left_x + 14, 19, "Dep", UiStyle::Label);
    canvas.text_style(left_x + 18, 19, &format!("{:.2}", state.synth.lfo_depth), UiStyle::Value);

    // FX section - row 20: Filter
    canvas.text_style(left_x + 2, 20, "Filt", UiStyle::Label);
    canvas.text_style(
        left_x + 7,
        20,
        if state.synth.filter_on { "ON" } else { "OFF" },
        if state.synth.filter_on { UiStyle::Active } else { UiStyle::Hint },
    );
    canvas.text_style(left_x + 11, 20, "Cut", UiStyle::Label);
    canvas.text_style(
        left_x + 15,
        20,
        &format!("{:.2}", state.synth.cutoff),
        if state.synth.filter_on { UiStyle::Filter } else { UiStyle::Value },
    );
    canvas.text_style(left_x + 20, 20, "Res", UiStyle::Label);
    canvas.text_style(
        left_x + 24,
        20,
        &format!("{:.2}", state.synth.resonance),
        if state.synth.filter_on { UiStyle::Filter } else { UiStyle::Value },
    );

    // FX section - row 21: Drive and Delay
    canvas.text_style(left_x + 2, 21, "Drv", UiStyle::Label);
    canvas.text_style(left_x + 6, 21, &format!("{:.2}", state.synth.fx.drive), UiStyle::Value);
    canvas.text_style(left_x + 11, 21, "Dly", UiStyle::Label);
    canvas.text_style(
        left_x + 15,
        21,
        &format!("{:.1}/{:.1}/{:.1}", state.synth.fx.delay_mix, state.synth.fx.delay_feedback, state.synth.fx.delay_time),
        UiStyle::Value,
    );

    // FX section - row 22: Warmth, Air, Reverb
    canvas.text_style(left_x + 2, 22, "Wrm", UiStyle::Label);
    canvas.text_style(left_x + 6, 22, &format!("{:.2}", state.synth.fx.warmth), UiStyle::Value);
    canvas.text_style(left_x + 11, 22, "Air", UiStyle::Label);
    canvas.text_style(left_x + 15, 22, &format!("{:.2}", state.synth.fx.air), UiStyle::Value);
    canvas.text_style(left_x + 20, 22, "Rev", UiStyle::Label);
    canvas.text_style(left_x + 24, 22, &format!("{:.2}", state.synth.fx.reverb), UiStyle::Value);

    // SAMPLE panel (full edit: Settings S → MAIN below Reverb)
    let smp_inner = left_w.saturating_sub(4).max(8);
    let rec_time = if state.sample.has_audio() {
        let el = state.sample.effective_len();
        let sec = el as f32 / state.sample.sample_rate.max(1) as f32;
        format!("{:05.2}s", sec)
    } else {
        "00.00s".to_string()
    };
    let rec_fill = if state.sample.input_recording {
        0.65
    } else if state.sample.has_audio() {
        (state.sample.effective_len() as f32 / (state.sample.sample_rate.max(1) as f32 * 45.0)).min(1.0)
    } else {
        0.0
    };
    let smp_bar_w = smp_inner.saturating_sub(16).clamp(4, 14);
    let rec_row = if state.sample.input_recording {
        format!("REC ● {:>7} {}", rec_time, bar(rec_fill, smp_bar_w))
    } else {
        format!(
            "REC idle {:>7} {}",
            rec_time,
            bar(rec_fill, smp_bar_w)
        )
    };
    canvas.text_style(
        left_x + 2,
        sample_top + 1,
        &short_label(&rec_row, smp_inner),
        if state.sample.input_recording {
            UiStyle::Filter
        } else {
            UiStyle::Value
        },
    );
    let ruler_w = smp_inner.saturating_sub(2).max(6);
    canvas.text_style(
        left_x + 2,
        sample_top + 2,
        &short_label(
            &sample_trim_ruler(state.sample.trim_start, state.sample.trim_end, ruler_w),
            smp_inner,
        ),
        UiStyle::Hint,
    );
    canvas.text_style(
        left_x + 2,
        sample_top + 3,
        &short_label(
            &format!(
                "S {:>3}%   E {:>3}%",
                (state.sample.trim_start * 100.0) as i32,
                (state.sample.trim_end * 100.0) as i32,
            ),
            smp_inner,
        ),
        UiStyle::Label,
    );
    let smp_status = if state.sample.input_recording {
        "recording from SOUND input…".to_string()
    } else if !state.sample.has_audio() {
        "no sample loaded".to_string()
    } else {
        format!(
            "{} {:+.1}st  G{:.2} Sp{:.2} {} {}",
            note_name(state.sample.root_midi as i16),
            state.sample.pitch_semitones,
            state.sample.gain,
            state.sample.speed,
            if state.sample.play_enabled { "PLAY" } else { "MUTE" },
            if matches!(state.ui.tab_focus, TabFocus::Sample) {
                "◀keys"
            } else {
                ""
            },
        )
    };
    canvas.text_style(
        left_x + 2,
        sample_top + 4,
        &short_label(&smp_status, smp_inner),
        if state.sample.has_audio() || state.sample.input_recording {
            UiStyle::Value
        } else {
            UiStyle::Hint
        },
    );
    canvas.text_style(
        left_x + 2,
        sample_top + 5,
        &short_label(
            "S:MAIN  TAB:SMP  ←→:±½st  ↑↓:gain  root:S→MAIN",
            smp_inner,
        ),
        UiStyle::Hint,
    );
    canvas.text_style(
        left_x + 2,
        sample_top + 6,
        &short_label("REC: S→MAIN; macOS: mic privacy + SOUND→Built-in if Default silent", smp_inner),
        UiStyle::Hint,
    );

    // SONG section: pattern / chain / hints (y offsets follow `song_top`)
    canvas.text_style(left_x + 2, song_top + 1, "PAT", UiStyle::Label);
    for p in 0..NUM_PATTERNS {
        let is_current = p == state.drums.current_pattern;
        let label = if is_current {
            format!("[{}]", p + 1)
        } else {
            format!(" {} ", p + 1)
        };
        canvas.text_style(
            left_x + 6 + (p * 3),
            song_top + 1,
            &label,
            if is_current { UiStyle::Active } else { UiStyle::Value },
        );
    }

    // SONG section - chain sequence (plain \ = list-play on/off; | or Shift+\ = append bank)
    canvas.text_style(left_x + 2, song_top + 2, "LIST", UiStyle::Label);
    canvas.text_style(
        left_x + 8,
        song_top + 2,
        if state.drums.chain_mode { "ON " } else { "OFF" },
        if state.drums.chain_mode { UiStyle::Active } else { UiStyle::Hint },
    );
    // Display chain sequence (fits in remaining width)
    let chain_display_width = left_w.saturating_sub(18);
    let chain_str: String = if state.drums.chain.is_empty() {
        "-- empty --".to_string()
    } else {
        state.drums.chain.iter()
            .enumerate()
            .map(|(i, p)| {
                if state.drums.chain_mode && i == state.drums.chain_position {
                    format!("[{}]", p + 1)
                } else {
                    format!("{}", p + 1)
                }
            })
            .collect::<Vec<_>>()
            .join("→")
    };
    canvas.text_style(
        left_x + 13,
        song_top + 2,
        &short_label(&chain_str, chain_display_width),
        UiStyle::Value,
    );

    // SONG section - hints
    canvas.text_style(
        left_x + 2,
        song_top + 3,
        &short_label("1-8 bank  Sh+1/2 fill  {/}  | add  BS pop  Del clr  \\ list", left_w.saturating_sub(4)),
        UiStyle::Hint,
    );

    let visual_render_h = visual_box_h.saturating_sub(4).max(1);
    
    // Resize framebuffer if dimensions changed
    let (fb_w, fb_h) = ui.visual_fb.size();
    let fb_resized = fb_w != visual_width as u16 || fb_h != visual_render_h as u16;
    if fb_resized {
        ui.visual_fb = Framebuffer::new(visual_width as u16, visual_render_h as u16);
    }
    
    // Track if we're using framebuffer-based visual (needs colored rendering)
    let using_framebuffer_visual = !matches!(state.ui.visual_mode, VisualMode::Camera | VisualMode::Scope);
    
    // Clear framebuffer and colored overlay when not using framebuffer visuals
    // This prevents artifacts when switching from Cube/etc to Scope
    if !using_framebuffer_visual {
        ui.visual_fb.clear();
        ui.visual_colored_lines.clear();
    }
    
    let visual_lines = match state.ui.visual_mode {
        VisualMode::Camera => {
            runtime.render_camera_ascii(visual_width, visual_render_h)
        }
        VisualMode::Scope => {
            let scope = if state.ui.scope_show_drums {
                &state.audio.recent_scope
            } else {
                &state.audio.recent_scope_synth
            };
            render_scope_braille(scope, visual_width, visual_render_h)
        }
        _ => {
            // Map VisualMode to registry index (effects 0-9)
            let effect_idx = match state.ui.visual_mode {
                VisualMode::Plasma => 0,
                VisualMode::Kaleidoscope => 1,
                VisualMode::MatrixRain => 2,
                VisualMode::Fire => 3,
                VisualMode::Tunnel => 4,
                VisualMode::Donut => 5,
                VisualMode::Fireworks => 6,
                VisualMode::Ripples => 7,
                VisualMode::Radio => 8,
                VisualMode::Cube => 9,
                _ => 0,
            };
            ui.visuals.set_index(effect_idx);
            if let Some(v) = ui.visuals.current_mut() {
                // Always sync effect dimensions with framebuffer (cheap operation)
                v.resize(visual_width as u16, visual_render_h as u16);
                // Set theme-based base color for effects that use it
                let (r, g, b) = theme_rgb(state.ui.theme);
                v.set_base_color(r, g, b);
                if matches!(state.ui.visual_mode, VisualMode::Donut) {
                    let _ = v.set_param("kick_swell", ParamValue::Float(state.ui.donut_kick_swell));
                }
                if matches!(state.ui.visual_mode, VisualMode::Cube) {
                    let _ = v.set_param("kick_punch", ParamValue::Float(state.ui.cube_kick_punch));
                    let _ = v.set_param("hat_rewind", ParamValue::Float(state.ui.cube_hat_rewind));
                }
                v.tick(1.0 / 60.0, &state.audio.reactive);
                v.render(&mut ui.visual_fb);
            }

            // Store colored lines for overlay (preserves per-char colors)
            ui.visual_colored_lines = ui.visual_fb.to_colored_strings();
            ui.visual_pos = (right_x + 1, 3);
            
            // Return STATIC spaces for canvas - prevents flicker from diff rewriting
            // The overlay will draw the actual colored content on top
            vec![" ".repeat(visual_width); visual_render_h]
        }
    };
    
    // Render visual lines to canvas
    // For framebuffer visuals: static spaces (overlay draws colors)
    // For scope/camera: actual content with theme color
    let visual_style = if using_framebuffer_visual { UiStyle::Plain } else { UiStyle::Scope };
    for (i, line) in visual_lines.iter().enumerate() {
        canvas.text_style(right_x + 1, 3 + i, line, visual_style);
    }
    
    canvas.text(
        right_x + 1,
        3 + visual_render_h,
        &format!(
            "AUDIO {}",
            short_label(&state.audio.status, right_w.saturating_sub(8))
        ),
    );
    canvas.text(
        right_x + 1,
        4 + visual_render_h,
        &format!(
            "MIDI  {}",
            short_label(&state.midi.status, right_w.saturating_sub(8))
        ),
    );

    let step_x0 = 6;
    let step_cell_w = 3;
    for step_idx in 0..NUM_STEPS {
        let label = format!("{:02}", step_idx + 1);
        let x = step_x0 + step_idx * step_cell_w;
        if x + 1 < term_w {
            // Highlight every 4th step: 0, 4, 8, 12, 16, 20, 24, 28 (1-indexed: 1, 5, 9, 13, 17, 21, 25, 29)
            let step_style = if step_idx % 4 == 0 {
                UiStyle::Hint  // Yellow for 1, 5, 9, 13, 17, 21, 25, 29
            } else {
                UiStyle::Value
            };
            canvas.text_style(
                x,
                drum_top + 1,
                &label,
                if step_idx == state.drums.current_step && state.drums.running {
                    UiStyle::Active
                } else {
                    step_style
                },
            );
        }
    }

    let editing_pattern = &state.drums.patterns[state.drums.current_pattern];
    for (idx, row) in editing_pattern.iter().enumerate() {
        let row_selected = matches!(state.ui.tab_focus, TabFocus::Drums) && idx == ui.drum_voice;
        canvas.text_style(
            3,
            drum_top + 2 + idx,
            ["KICK", "SNRE", "CLAP", "HHAT", "TOM ", "CYMB"][idx],
            if row_selected {
                UiStyle::Cursor
            } else {
                UiStyle::Label
            },
        );
        let mut x = step_x0;
        for (step_idx, enabled) in row.iter().enumerate() {
            let ch = if *enabled { '■' } else { '·' };
            let is_selected = row_selected && step_idx == ui.drum_step;
            let is_playhead = state.drums.running && step_idx == state.drums.current_step;
            let cell = if is_selected {
                format!("[{ch}]")
            } else {
                format!(" {ch} ")
            };
            canvas.text_style(
                x,
                drum_top + 2 + idx,
                &cell,
                if row_selected || is_selected {
                    UiStyle::Cursor
                } else if is_playhead {
                    UiStyle::Active
                } else {
                    UiStyle::Value
                },
            );
            x += step_cell_w;
        }
        canvas.text_style(
            term_w.saturating_sub(15),
            drum_top + 2 + idx,
            &format!("{:>3}%", (state.drums.volumes[idx] * 100.0) as i32),
            if row_selected {
                UiStyle::Cursor
            } else {
                UiStyle::Value
            },
        );
    }

    canvas.text(
        5,
        drum_top + 8,
        &short_label(
            &format!(
                "BPM {:.1}  RUN {}  STEP {:02}  {}  WAV {}  LAST {}",
                state.drums.bpm,
                if state.drums.running { "ON " } else { "OFF" },
                state.drums.current_step + 1,
                get_bank(state.drums.bank).name,
                if state.audio.global_recording.recording {
                    "REC"
                } else {
                    "OFF"
                },
                state
                    .audio
                    .global_recording
                    .last_path
                    .as_deref()
                    .unwrap_or("--")
            ),
            term_w.saturating_sub(8),
        ),
    );
    let footer = if let Some(notice) = drum_notice_text(ui) {
        short_label(notice, term_w.saturating_sub(8))
    } else if state.ui.settings_open {
        "Row 1 switches page. Use [ and ] to page through tabs. ↑↓ select, ←→ change, Enter/Space run. S/Esc close."
            .to_string()
    } else if matches!(state.ui.tab_focus, TabFocus::Synth) {
        "TAB | ←→ pitch ↑↓ vol | a..k notes | R/T/Y/P/U loop | \\ | Sh+G WAV | S settings".to_string()
    } else if matches!(state.ui.tab_focus, TabFocus::Sample) {
        "TAB | ←→ tune ±½st ↑↓ gain | a..k notes | R/T/Y/P/U loop | Sh+G WAV | S settings (Smp root)".to_string()
    } else {
        "TAB | ←→ step ↑↓ voice | [ ] vol | SPC | r run | Sh+G WAV | , . BPM | { } pat | S settings".to_string()
    };
    canvas.text(5, footer_y, &footer);

    if state.ui.settings_open {
        let lines = settings_lines(&state, ui);
        let content_w = lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0)
            + 4;
        let box_w = content_w
            .max(52)
            .min(term_w.saturating_sub(4))
            .min(88);
        let max_box_h = term_h.saturating_sub(2).max(8);
        let desired_box_h = lines.len() + 5;
        let box_h = desired_box_h.clamp(8, max_box_h);
        let box_x = (term_w.saturating_sub(box_w)) / 2;
        let box_y = (term_h.saturating_sub(box_h)) / 2;
        let inner_max_x = box_x + box_w - 1;
        canvas.fill_rect(box_x + 1, box_y + 1, box_w - 2, box_h - 2, UiStyle::Backdrop);
        canvas.boxed_style(
            box_x,
            box_y,
            box_w,
            box_h,
            &format!(" SETTINGS {:?} ", state.ui.settings_page),
            UiStyle::Scope,
        );
        canvas.text_clipped(
            box_x + 2,
            box_y + 1,
            &settings_tabs(state.ui.settings_page, box_w.saturating_sub(4)),
            UiStyle::Header,
            inner_max_x,
        );
        let visible_rows = box_h.saturating_sub(5);
        let start_row = if lines.len() <= visible_rows {
            0
        } else {
            let half = visible_rows / 2;
            ui.settings_cursor
                .saturating_sub(half)
                .min(lines.len().saturating_sub(visible_rows))
        };
        let total_lines = lines.len();
        for (idx, line) in lines.iter().skip(start_row).take(visible_rows).enumerate() {
            canvas.text_clipped(
                box_x + 2,
                box_y + 2 + idx,
                &short_label(line, box_w.saturating_sub(4)),
                if line.starts_with('>') {
                    UiStyle::Cursor
                } else {
                    UiStyle::Backdrop
                },
                inner_max_x,
            );
        }
        if start_row > 0 {
            canvas.text_clipped(box_x + box_w.saturating_sub(4), box_y + 1, "↑", UiStyle::Backdrop, inner_max_x);
        }
        if start_row + visible_rows < total_lines {
            canvas.text_clipped(box_x + box_w.saturating_sub(4), box_y + box_h - 3, "↓", UiStyle::Backdrop, inner_max_x);
        }
        canvas.text_clipped(
            box_x + 2,
            box_y + box_h - 2,
            &short_label("[/] tabs, ↑↓ select, ←→ change, Enter run. S/Esc close.", box_w.saturating_sub(4)),
            UiStyle::Backdrop,
            inner_max_x,
        );
    }

    let help_overlay = state
        .ui
        .help_open
        .then(|| compute_help_overlay(term_w, term_h));
    if let Some((help_rows, box_w, box_h, box_x, box_y, inner_max_x)) = &help_overlay {
        canvas.fill_rect(box_x + 1, box_y + 1, box_w - 2, box_h - 2, UiStyle::Backdrop);
        draw_help_overlay(
            &mut canvas,
            *box_x,
            *box_y,
            *box_w,
            *box_h,
            *inner_max_x,
            ui,
            help_rows,
        );
    }

    // Set overlay mask for settings/help panel so visual doesn't cover them
    ui.overlay_mask = None;
    if state.ui.settings_open {
        let lines = settings_lines(&state, ui);
        let content_w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) + 4;
        let box_w = content_w.max(52).min(term_w.saturating_sub(4)).min(88);
        let max_box_h = term_h.saturating_sub(2).max(8);
        let desired_box_h = lines.len() + 5;
        let box_h = desired_box_h.clamp(8, max_box_h);
        let box_x = (term_w.saturating_sub(box_w)) / 2;
        let box_y = (term_h.saturating_sub(box_h)) / 2;
        ui.overlay_mask = Some((box_x, box_y, box_w, box_h));
    } else if let Some((_, box_w, box_h, box_x, box_y, _)) = &help_overlay {
        ui.overlay_mask = Some((*box_x, *box_y, *box_w, *box_h));
    }

    Ok(canvas.finish(state.ui.theme))
}

struct Canvas {
    width: usize,
    height: usize,
    cells: Vec<Vec<PaintCell>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PaintCell {
    ch: char,
    style: UiStyle,
}

#[derive(Clone)]
struct RenderedFrame {
    cells: Vec<Vec<PaintCell>>,
    theme: Theme,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UiStyle {
    Plain,
    Backdrop,
    Header,
    Label,
    Value,
    Active,
    Bar,
    Hint,
    Scope,
    Cursor,
    Filter,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![
                vec![
                    PaintCell {
                        ch: ' ',
                        style: UiStyle::Plain
                    };
                    width
                ];
                height
            ],
        }
    }

    fn text(&mut self, x: usize, y: usize, text: &str) {
        self.text_style(x, y, text, UiStyle::Value);
    }

    fn text_style(&mut self, x: usize, y: usize, text: &str, style: UiStyle) {
        if y >= self.height {
            return;
        }
        for (idx, ch) in text.chars().enumerate() {
            if x + idx >= self.width {
                break;
            }
            self.cells[y][x + idx] = PaintCell { ch, style };
        }
    }

    fn text_center_style(&mut self, y: usize, text: &str, style: UiStyle) {
        let width = text.chars().count();
        let x = self.width.saturating_sub(width) / 2;
        self.text_style(x, y, text, style);
    }

    fn text_clipped(&mut self, x: usize, y: usize, text: &str, style: UiStyle, max_x: usize) {
        if y >= self.height {
            return;
        }
        for (idx, ch) in text.chars().enumerate() {
            let cx = x + idx;
            if cx >= max_x || cx >= self.width {
                break;
            }
            self.cells[y][cx] = PaintCell { ch, style };
        }
    }

    fn fill_rect(&mut self, x: usize, y: usize, width: usize, height: usize, style: UiStyle) {
        if width == 0 || height == 0 || x >= self.width || y >= self.height {
            return;
        }
        let x_end = (x + width).min(self.width);
        let y_end = (y + height).min(self.height);
        for row in self.cells.iter_mut().take(y_end).skip(y) {
            for cell in row.iter_mut().take(x_end).skip(x) {
                cell.ch = ' ';
                cell.style = style;
            }
        }
    }

    fn boxed_style(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        title: &str,
        style: UiStyle,
    ) {
        if width < 2 || height < 2 || x >= self.width || y >= self.height {
            return;
        }
        let x2 = (x + width - 1).min(self.width - 1);
        let y2 = (y + height - 1).min(self.height - 1);
        self.cells[y][x] = PaintCell { ch: '┌', style };
        self.cells[y][x2] = PaintCell { ch: '┐', style };
        self.cells[y2][x] = PaintCell { ch: '└', style };
        self.cells[y2][x2] = PaintCell { ch: '┘', style };
        for cx in x + 1..x2 {
            self.cells[y][cx] = PaintCell { ch: '─', style };
            self.cells[y2][cx] = PaintCell { ch: '─', style };
        }
        for cy in y + 1..y2 {
            self.cells[cy][x] = PaintCell { ch: '│', style };
            self.cells[cy][x2] = PaintCell { ch: '│', style };
        }
        self.text_style(x + 2, y, title, style);
    }

    fn finish(self, theme: Theme) -> RenderedFrame {
        RenderedFrame {
            cells: self.cells,
            theme,
        }
    }
}

fn style_seq(theme: Theme, style: UiStyle) -> &'static str {
    match style {
        UiStyle::Plain => "\x1b[0m",
        UiStyle::Backdrop => "\x1b[37;48;5;236m",
        UiStyle::Header => theme_header_seq(theme),
        UiStyle::Label => "\x1b[36;49m",
        UiStyle::Value => "\x1b[37;49m",
        UiStyle::Active => "\x1b[30;42;1m",
        UiStyle::Bar => "\x1b[32;49m",
        UiStyle::Hint => "\x1b[33;49m",
        UiStyle::Scope => theme_scope_seq(theme),
        UiStyle::Cursor => "\x1b[30;43;1m",
        UiStyle::Filter => "\x1b[31;49m",
    }
}

fn theme_scope_seq(theme: Theme) -> &'static str {
    match theme {
        Theme::Magenta => "\x1b[35;49m",
        Theme::Mint => "\x1b[36;49m",
        Theme::Amber => "\x1b[33;49m",
    }
}

fn theme_rgb(theme: Theme) -> (u8, u8, u8) {
    match theme {
        Theme::Magenta => (200, 100, 220),
        Theme::Mint => (100, 220, 200),
        Theme::Amber => (220, 180, 100),
    }
}

fn theme_header_seq(theme: Theme) -> &'static str {
    match theme {
        Theme::Magenta => "\x1b[30;45;1m",
        Theme::Mint => "\x1b[30;46;1m",
        Theme::Amber => "\x1b[30;43;1m",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrumClearConfirm {
    Row(usize),
    All,
}

struct UiLocalState {
    drum_voice: usize,
    drum_step: usize,
    keyboard_note: Option<i8>,
    keyboard_note_repeat_count: u32,
    keyboard_note_started_at: Option<Instant>,
    keyboard_note_last_repeat_at: Option<Instant>,
    keyboard_note_released_at: Option<Instant>,
    drum_clear_confirm: Option<DrumClearConfirm>,
    drum_notice: Option<String>,
    drum_notice_until: Option<Instant>,
    settings_cursor: usize,
    project_index: usize,
    visuals: VisualRegistry,
    visual_fb: Framebuffer,
    /// Colored visual lines for framebuffer-based effects (with embedded ANSI codes)
    visual_colored_lines: Vec<String>,
    /// Position (x, y) where visual lines should be rendered
    visual_pos: (usize, usize),
    /// Mask region where visual overlay should NOT draw (settings/help box): (x, y, w, h)
    overlay_mask: Option<(usize, usize, usize, usize)>,
    /// First visible body line index in the help overlay (see `draw_help_overlay`).
    help_scroll: usize,
}

impl Default for UiLocalState {
    fn default() -> Self {
        Self {
            drum_voice: 0,
            drum_step: 0,
            keyboard_note: None,
            keyboard_note_repeat_count: 0,
            keyboard_note_started_at: None,
            keyboard_note_last_repeat_at: None,
            keyboard_note_released_at: None,
            drum_clear_confirm: None,
            drum_notice: None,
            drum_notice_until: None,
            settings_cursor: 0,
            project_index: 0,
            visuals: VisualRegistry::new(),
            visual_fb: Framebuffer::new(80, 20),
            visual_colored_lines: Vec::new(),
            visual_pos: (0, 0),
            overlay_mask: None,
            help_scroll: 0,
        }
    }
}

/// `Shift+S` as `Char('S')` (legacy) or `Char('s')` + [`KeyModifiers::SHIFT`] (Kitty
/// `REPORT_ALL_KEYS_AS_ESCAPE_CODES`). Plain `s` / `h` must not match.
fn key_matches_shifted_base_letter(key: &KeyEvent, base: char) -> bool {
    let lo = base.to_ascii_lowercase();
    let hi = base.to_ascii_uppercase();
    match key.code {
        KeyCode::Char(c) if c == hi => true,
        KeyCode::Char(c) if c == lo => key.modifiers.contains(KeyModifiers::SHIFT),
        _ => false,
    }
}

/// US-style digit row: `Char('@')` etc. or `Char('2')` + Shift. Plain digit is excluded by caller guards.
fn key_shifted_digit_row(key: &KeyEvent, digit: char, shifted_symbol: char) -> bool {
    match key.code {
        KeyCode::Char(c) if c == shifted_symbol => true,
        KeyCode::Char(c) if c == digit => key.modifiers.contains(KeyModifiers::SHIFT),
        _ => false,
    }
}

/// QWERTY chromatic row (A W S E D F …) — `offset` = semitones from `root` / `base_midi`.
/// Matches **case-insensitively** so Caps Lock / uppercase key events still work.
fn key_to_offset(code: &KeyCode) -> Option<i8> {
    let ch = match code {
        KeyCode::Char(c) if c.is_ascii_alphabetic() => c.to_ascii_lowercase(),
        _ => return None,
    };
    match ch {
        'a' => Some(0),
        'w' => Some(1),
        's' => Some(2),
        'e' => Some(3),
        'd' => Some(4),
        'f' => Some(5),
        't' => Some(6),
        'g' => Some(7),
        'y' => Some(8),
        'h' => Some(9),
        'u' => Some(10),
        'j' => Some(11),
        'k' => Some(12),
        _ => None,
    }
}

fn cycle_waveform(state: &mut AppState, delta: i32) {
    let osc = &mut state.synth.oscillators[state.synth.active_osc];
    let index = match osc.waveform {
        Waveform::Sine => 0,
        Waveform::Triangle => 1,
        Waveform::Square => 2,
        Waveform::Saw => 3,
    };
    osc.waveform = match (index + delta).rem_euclid(4) {
        0 => Waveform::Sine,
        1 => Waveform::Triangle,
        2 => Waveform::Square,
        _ => Waveform::Saw,
    };
}

fn note_name(midi: i16) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = midi.clamp(0, 127);
    format!("{}{}", NAMES[(midi % 12) as usize], midi / 12 - 1)
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Magenta => "MAGENTA",
        Theme::Mint => "MINT",
        Theme::Amber => "AMBER",
    }
}

fn drum_notice_text(ui: &UiLocalState) -> Option<&str> {
    let until = ui.drum_notice_until?;
    if Instant::now() < until {
        ui.drum_notice.as_deref()
    } else {
        None
    }
}

fn apply_pattern(state: &mut AppState, pattern: usize) {
    let p = state.drums.current_pattern;
    state.drums.patterns[p] = [[false; 32]; 6];
    match pattern {
        0 => {
            for step in [0, 8, 16, 24] {
                state.drums.patterns[p][0][step] = true;
            }
            for step in [4, 12, 20, 28] {
                state.drums.patterns[p][1][step] = true;
            }
            for step in (0..32).step_by(2) {
                state.drums.patterns[p][3][step] = true;
            }
        }
        _ => {
            for step in [0, 11, 16, 24] {
                state.drums.patterns[p][0][step] = true;
            }
            for step in [4, 12, 20, 28] {
                state.drums.patterns[p][1][step] = true;
            }
            for step in [7, 15, 23, 31] {
                state.drums.patterns[p][4][step] = true;
            }
        }
    }
}

fn next_page(page: SettingsPage) -> SettingsPage {
    match page {
        SettingsPage::Main => SettingsPage::Visuals,
        SettingsPage::Visuals => SettingsPage::Project,
        SettingsPage::Project => SettingsPage::SoundDevice,
        SettingsPage::SoundDevice => SettingsPage::Midi,
        SettingsPage::Midi => SettingsPage::Main,
    }
}

fn prev_page(page: SettingsPage) -> SettingsPage {
    match page {
        SettingsPage::Main => SettingsPage::Midi,
        SettingsPage::Visuals => SettingsPage::Main,
        SettingsPage::Project => SettingsPage::Visuals,
        SettingsPage::SoundDevice => SettingsPage::Project,
        SettingsPage::Midi => SettingsPage::SoundDevice,
    }
}

fn settings_row_count(page: SettingsPage, visual_mode: VisualMode) -> usize {
    match page {
        SettingsPage::Main => 22,
        SettingsPage::Visuals => match visual_mode {
            VisualMode::Scope => 2,
            VisualMode::Donut => 2,
            VisualMode::Cube => 3,
            VisualMode::Camera => 3,
            _ => 1,
        },
        SettingsPage::Project => 4,
        SettingsPage::SoundDevice => 4,
        SettingsPage::Midi => 14,
    }
}

fn settings_tabs(page: SettingsPage, width: usize) -> String {
    let names = [
        "MAIN",
        "VISUALS",
        "PROJECT",
        "SOUND",
        "MIDI",
    ];
    let pages = [
        SettingsPage::Main,
        SettingsPage::Visuals,
        SettingsPage::Project,
        SettingsPage::SoundDevice,
        SettingsPage::Midi,
    ];
    let mut out = String::new();
    for (idx, name) in names.iter().enumerate() {
        let token = if pages[idx] == page {
            format!("[{}] ", name)
        } else {
            format!(" {}  ", name)
        };
        out.push_str(&token);
    }
    short_label(&out, width)
}

const HELP_BODY_VIEWPORT_CAP: usize = 22;

enum HelpPaintRow {
    SectionTitle(String),
    Blank,
    KeyLine {
        key: String,
        desc: String,
        key_col_w: usize,
    },
}

fn compute_help_overlay(
    term_w: usize,
    term_h: usize,
) -> (
    Vec<HelpPaintRow>,
    usize,
    usize,
    usize,
    usize,
    usize,
) {
    let box_w = term_w.saturating_sub(4).min(100).max(70);
    let inner_w = box_w.saturating_sub(4).max(20);
    let help_rows = build_help_paint_rows(inner_w);
    let body_visible = help_rows.len().min(HELP_BODY_VIEWPORT_CAP).max(1);
    let box_h = (body_visible + 6)
        .min(term_h.saturating_sub(4))
        .max(7);
    let box_x = (term_w.saturating_sub(box_w)) / 2;
    let box_y = (term_h.saturating_sub(box_h)) / 2;
    let inner_max_x = box_x + box_w - 1;
    (help_rows, box_w, box_h, box_x, box_y, inner_max_x)
}

fn build_help_paint_rows(inner_w: usize) -> Vec<HelpPaintRow> {
    let sections: [(&str, Vec<(&str, &str)>); 5] = [
        (
            "PLAY NOTES",
            vec![
                (
                    "[a w s e d f t g y h u j k]",
                    "Chromatic keys: semitone offsets from synth base or sample anchor (S→MAIN Smp root).",
                ),
                (
                    "[MIDI keys]",
                    "External MIDI notes play synth.",
                ),
                (
                    "[TAB]",
                    "Switch between Synth / Drums / Sample modes.",
                ),
                (
                    "[←] [→]",
                    "Synth: base pitch. Sample: transpose ±½ st. Drums: step.",
                ),
                (
                    "[↑] [↓]",
                    "Volume: synth / sample gain. Drums: move voice (instrument row).",
                ),
                ("[[ ] []]", "Drums: row volume down / up."),
                ("[SPC]", "Release held note (synth/sample)."),
            ],
        ),
        (
            "OSC + SHAPE",
            vec![
                ("[1] [2]", "Select osc 1 or 2."),
                ("[z] [x]", "Change selected waveform."),
                ("[↑] [↓]", "Synth master volume."),
                (
                    "[ [ ] ] [ { } ]",
                    "Attack and Release",
                ),
                ("[m]", "Toggle gate/free mode."),
            ],
        ),
        (
            "MOD + FILTER + FX",
            vec![
                (
                    "[l] [o]",
                    "LFO wave and target.",
                ),
                ("[,] [.] [;] [']", "LFO rate and depth."),
                (
                    "[p] [-] [=] [_] [+]",
                    "Plain p: filter on/off. [-] [=] cutoff, [_] [+] resonance. Shift+P is loop play (Synth/Sample).",
                ),
                (
                    "[D] [F] [J] [K] [N] [M] [V] [B]",
                    "Drive, delay, feedback, time.",
                ),
                ("[W] [A] [E]", "Warmth, air, reverb."),
                (
                    "[S] MAIN ↓ past Rev",
                    "Sample: record from SOUND-tab input, trim, keys+MIDI when Play ON.",
                ),
            ],
        ),
        (
            "LOOP + DRUMS",
            vec![
                (
                    "[R] [T] [Y] [P] [U]",
                    "Shift+letter: Synth tab → main synth loop; Sample tab → sample performance loop (separate buffers). Drums: no loop.",
                ),
                (
                    "[Shift+G] / uppercase G",
                    "Global mix record to WAV from any tab. On Synth/Sample, plain g is still a chromatic key.",
                ),
                (
                    "[Drums]",
                    "←→ step, ↑↓ voice, [ ] row volume.",
                ),
                (
                    "[\\] [|]",
                    "Drums: \\ toggles play-through-pattern-list; | or Shift+\\ appends current bank to that list.",
                ),
                (
                    "[SPC] [r] [c] [X] [1-8] [Sh+1/2]",
                    "Drums: 1-8 select pattern bank; Shift+1/2 load starter into current bank; step/run/clear.",
                ),
                ("[Settings]", "S / Esc — [ ] pages, arrows adjust row."),
            ],
        ),
        (
            "LOOP EDITING",
            vec![
                ("[I] [O]", "Loop gain up/down (Synth or Sample tab)."),
                (
                    "Shift+2 @ / Shift+3 #",
                    "Trim loop start forward / back.",
                ),
                (
                    "Shift+4 $ / Shift+5 %",
                    "Trim loop end forward / back.",
                ),
                (
                    "Shift+6 ^ / Shift+7 &",
                    "Time-stretch faster / slower (0.25x–4x).",
                ),
            ],
        ),
    ];

    let mut out: Vec<HelpPaintRow> = Vec::new();
    for (title, rows) in sections.iter() {
        out.push(HelpPaintRow::SectionTitle(format!("* {title}")));
        let max_kw = rows
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(0)
            .min(inner_w.saturating_sub(12))
            .max(6);
        for (keys, desc) in rows.iter() {
            let desc_w = inner_w.saturating_sub(max_kw + 2).max(1);
            let wrapped = wrap_text(desc, desc_w);
            for (i, line) in wrapped.iter().enumerate() {
                out.push(HelpPaintRow::KeyLine {
                    key: if i == 0 {
                        (*keys).to_string()
                    } else {
                        String::new()
                    },
                    desc: line.clone(),
                    key_col_w: max_kw,
                });
            }
        }
        out.push(HelpPaintRow::Blank);
    }
    out
}

fn draw_help_overlay(
    canvas: &mut Canvas,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    max_x: usize,
    ui: &mut UiLocalState,
    help_rows: &[HelpPaintRow],
) {
    canvas.boxed_style(x, y, width, height, " MUSH HELP ", UiStyle::Scope);
    let inner_w = width.saturating_sub(4).max(20);
    // Rows reserved above the scroll region (title + separator + footer).
    let header_rows = 4usize;
    let footer_rows = 2usize;
    let body_h = height.saturating_sub(header_rows + footer_rows).max(1);

    canvas.text_clipped(
        x + 2,
        y + 1,
        &short_label("Terminal synth + drums + looper", inner_w),
        UiStyle::Backdrop,
        max_x,
    );
    if width >= 72 {
        canvas.text_clipped(
            x + width.saturating_sub(12),
            y + 1,
            "H close",
            UiStyle::Backdrop,
            max_x,
        );
    } else {
        canvas.text_clipped(x + 2, y + 2, "H close", UiStyle::Backdrop, max_x);
    }
    let sep_y = y + 3;
    canvas.text_clipped(
        x + 2,
        sep_y,
        &"─".repeat(inner_w.min(120)),
        UiStyle::Backdrop,
        max_x,
    );
    let body_top = sep_y + 1;

    let max_scroll = help_rows.len().saturating_sub(body_h);
    ui.help_scroll = ui.help_scroll.min(max_scroll);

    for (i, row) in help_rows
        .iter()
        .skip(ui.help_scroll)
        .take(body_h)
        .enumerate()
    {
        let rowy = body_top + i;
        match row {
            HelpPaintRow::SectionTitle(t) => {
                canvas.text_clipped(
                    x + 2,
                    rowy,
                    &short_label(t, inner_w),
                    UiStyle::Header,
                    max_x,
                );
            }
            HelpPaintRow::Blank => {}
            HelpPaintRow::KeyLine {
                key,
                desc,
                key_col_w,
            } => {
                if !key.is_empty() {
                    canvas.text_clipped(
                        x + 2,
                        rowy,
                        &short_label(key, *key_col_w),
                        UiStyle::Cursor,
                        max_x,
                    );
                }
                let desc_x = x + 2 + key_col_w + 2;
                let desc_w = inner_w.saturating_sub(*key_col_w + 2).max(1);
                canvas.text_clipped(
                    desc_x,
                    rowy,
                    &short_label(desc, desc_w),
                    UiStyle::Backdrop,
                    max_x,
                );
            }
        }
    }

    let footer = if max_scroll > 0 {
        format!(
            "Lines {}-{} of {} | Up/Down | Esc | H",
            ui.help_scroll + 1,
            (ui.help_scroll + body_h).min(help_rows.len()),
            help_rows.len()
        )
    } else {
        "Esc close | H help".to_string()
    };
    canvas.text_clipped(
        x + 2,
        y + height.saturating_sub(2),
        &short_label(&footer, inner_w),
        UiStyle::Backdrop,
        max_x,
    );
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let needs_space = !current.is_empty();
        let next_len =
            current.chars().count() + word.chars().count() + if needs_space { 1 } else { 0 };
        if next_len > width && !current.is_empty() {
            lines.push(current);
            current = String::new();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn settings_lines(state: &AppState, ui: &UiLocalState) -> Vec<String> {
    let selected = |idx: usize, text: String| {
        if idx == ui.settings_cursor {
            format!("> {text}")
        } else {
            format!("  {text}")
        }
    };

    match state.ui.settings_page {
        SettingsPage::Main => vec![
            selected(0, format!("Theme      {}", theme_name(state.ui.theme))),
            selected(1, format!("Drum bank  {}  [←→]", get_bank(state.drums.bank).name)),
            selected(2, format!("Voices     {}", state.synth.voices)),
            selected(3, format!("Edit osc   OSC{}", state.synth.active_osc + 1)),
            selected(
                4,
                format!("Waveform   {:?}", state.synth.oscillators[state.synth.active_osc].waveform),
            ),
            selected(
                5,
                format!("Level      {:>3}%", (state.synth.oscillators[state.synth.active_osc].level * 100.0) as i32),
            ),
            selected(
                6,
                format!("Octave     {:+}", state.synth.oscillators[state.synth.active_osc].octave),
            ),
            selected(
                7,
                format!("Detune     {:+5.1}c", state.synth.oscillators[state.synth.active_osc].detune_cents),
            ),
            selected(8, format!("Warmth     {:.2}", state.synth.fx.warmth)),
            selected(9, format!("Air        {:.2}", state.synth.fx.air)),
            selected(10, format!("Reverb     {:.2}", state.synth.fx.reverb)),
            selected(
                11,
                format!(
                    "Smp REC    {}  [ENTER]",
                    if state.sample.input_recording {
                        "●REC"
                    } else {
                        "idle"
                    }
                ),
            ),
            selected(12, "Smp CLEAR  [ENTER]".to_string()),
            selected(
                13,
                format!("Smp trim0  {:.0}%", state.sample.trim_start * 100.0),
            ),
            selected(
                14,
                format!("Smp trim1  {:.0}%", state.sample.trim_end * 100.0),
            ),
            selected(15, format!("Smp gain   {:.2}", state.sample.gain)),
            selected(16, format!("Smp speed  {:.2}", state.sample.speed)),
            selected(
                17,
                format!("Smp pitch  {:+.1} st", state.sample.pitch_semitones),
            ),
            selected(
                18,
                format!(
                    "Smp root   {} ({})",
                    note_name(state.sample.root_midi as i16),
                    state.sample.root_midi
                ),
            ),
            selected(19, format!("Smp atk    {:.3}s", state.sample.attack)),
            selected(20, format!("Smp rel    {:.3}s", state.sample.release)),
            selected(
                21,
                format!(
                    "Smp play   {}",
                    if state.sample.play_enabled { "ON " } else { "OFF" }
                ),
            ),
        ],
        SettingsPage::Visuals => {
            let mut rows = vec![selected(0, format!("Visual     {}", state.ui.visual_mode.name()))];
            match state.ui.visual_mode {
                VisualMode::Scope => {
                    rows.push(selected(
                        1,
                        format!(
                            "Drums scope {}",
                            if state.ui.scope_show_drums { "ON" } else { "OFF" }
                        ),
                    ));
                }
                VisualMode::Donut => {
                    rows.push(selected(
                        1,
                        format!("Kick swell {:.2}  [←→]", state.ui.donut_kick_swell),
                    ));
                }
                VisualMode::Cube => {
                    rows.push(selected(
                        1,
                        format!("Kick punch {:.2}  [←→]", state.ui.cube_kick_punch),
                    ));
                    rows.push(selected(
                        2,
                        format!("Hat rewind {:.2}  [←→]", state.ui.cube_hat_rewind),
                    ));
                }
                VisualMode::Camera => {
                    rows.push(selected(1, format!("FX Style   {:?}", state.ui.visual_fx)));
                    rows.push(selected(2, format!("FX Depth   {:.2}", state.ui.visual_fx_depth)));
                }
                _ => {}
            }
            rows
        }
        SettingsPage::Project => {
            let current = state
                .project
                .available
                .get(ui.project_index)
                .map(|p| p.name.as_str())
                .unwrap_or("(no projects)");
            let status = if state.project.status.is_empty() {
                "".to_string()
            } else {
                format!("  Status: {}", state.project.status)
            };
            vec![
                selected(0, format!("Project   {}", current)),
                selected(1, "Open  [ENTER]".to_string()),
                selected(2, "Save current  [ENTER]".to_string()),
                selected(3, "Save as new  [ENTER]".to_string()),
                status,
            ]
        }
        SettingsPage::SoundDevice => vec![
            selected(
                0,
                format!(
                    "Output    {}  [←→ ENTER=apply]",
                    format_audio_selection(&state.audio.output)
                ),
            ),
            selected(
                1,
                format!(
                    "Input     {}  [←→]",
                    format_audio_selection(&state.audio.input)
                ),
            ),
            selected(2, "Refresh list".to_string()),
            selected(3, format!("Status    {}", &state.audio.status)),
        ],
        SettingsPage::Midi => {
            let target = state.midi.selected_target();
            let binding = state.midi.bindings.get(&target).copied().flatten();
            vec![
                // Device settings
                selected(
                    0,
                    format!(
                        "MIDI on   {}",
                        if state.midi.enabled { "ON" } else { "OFF" }
                    ),
                ),
                selected(
                    1,
                    format!(
                        "Device    {}",
                        state.midi.device_name.as_deref().unwrap_or("None")
                    ),
                ),
                selected(2, "Refresh".to_string()),
                selected(
                    3,
                    format!(
                        "Channel   {}",
                        match state.midi.channel {
                            MidiChannel::All => "ALL".to_string(),
                            MidiChannel::Index(ch) => format!("{}", ch + 1),
                        }
                    ),
                ),
                selected(
                    4,
                    format!(
                        "Note in   {}",
                        if state.midi.note_input { "ON" } else { "OFF" }
                    ),
                ),
                selected(
                    5,
                    format!(
                        "Pad in    {}",
                        if state.midi.pad_input { "ON" } else { "OFF" }
                    ),
                ),
                // Note remap settings
                selected(
                    6,
                    format!(
                        "Src note  {} ({})",
                        note_name(state.midi.note_edit_in as i16),
                        state.midi.note_edit_in
                    ),
                ),
                selected(7, format!("Learn src {:?}", state.midi.learn_mode)),
                selected(
                    8,
                    format!(
                        "Dst note  {} ({})",
                        note_name(state.midi.note_edit_out as i16),
                        state.midi.note_edit_out
                    ),
                ),
                selected(
                    9,
                    format!("Save map  {} -> {}", state.midi.note_edit_in, state.midi.note_edit_out),
                ),
                selected(10, format!("Clear map {}", state.midi.note_edit_in)),
                // Binding settings
                selected(11, format!("Target    {}", target.label())),
                selected(12, format!("Learn bind {:?}", state.midi.learn_mode)),
                selected(
                    13,
                    format!("Clear bind {}", binding.map(|v| v.to_string()).unwrap_or_else(|| "--".to_string())),
                ),
            ]
        }
    }
}

fn adjust_setting(runtime: &mut Runtime, ui: &mut UiLocalState, delta: i32) -> Result<()> {
    let mut state = runtime.state.lock();
    match state.ui.settings_page {
        SettingsPage::Main => match ui.settings_cursor {
            0 => {
                state.ui.theme = cycle_theme(state.ui.theme, delta);
            }
            1 => state.drums.bank = ((state.drums.bank as i32 + delta).rem_euclid(NUM_DRUM_BANKS as i32)) as usize,
            2 => {
                state.synth.voices =
                    (state.synth.voices as i32 + delta).clamp(1, MAX_VOICES as i32) as u8
            }
            3 => {
                state.synth.active_osc =
                    (state.synth.active_osc as i32 + delta).rem_euclid(2) as usize
            }
            4 => cycle_waveform(&mut state, delta),
            5 => {
                let active_osc = state.synth.active_osc;
                let osc = &mut state.synth.oscillators[active_osc];
                osc.level = (osc.level + delta as f32 * 0.05).clamp(0.0, 1.0);
            }
            6 => {
                let active_osc = state.synth.active_osc;
                let osc = &mut state.synth.oscillators[active_osc];
                osc.octave = (osc.octave + delta as i8).clamp(-3, 3);
            }
            7 => {
                let active_osc = state.synth.active_osc;
                let osc = &mut state.synth.oscillators[active_osc];
                osc.detune_cents = (osc.detune_cents + delta as f32 * 1.0).clamp(-100.0, 100.0);
            }
            8 => {
                state.synth.fx.warmth =
                    (state.synth.fx.warmth + delta as f32 * 0.05).clamp(0.0, 1.0)
            }
            9 => state.synth.fx.air = (state.synth.fx.air + delta as f32 * 0.05).clamp(0.0, 1.0),
            10 => {
                state.synth.fx.reverb =
                    (state.synth.fx.reverb + delta as f32 * 0.05).clamp(0.0, 1.0)
            }
            11 | 12 => {}
            13 => {
                state.sample.trim_start =
                    (state.sample.trim_start + delta as f32 * 0.01).clamp(0.0, 0.9);
            }
            14 => {
                state.sample.trim_end =
                    (state.sample.trim_end + delta as f32 * 0.01).clamp(0.0, 0.9);
            }
            15 => {
                state.sample.gain = (state.sample.gain + delta as f32 * 0.04).clamp(0.0, 2.5);
            }
            16 => {
                state.sample.speed = (state.sample.speed + delta as f32 * 0.05).clamp(0.05, 8.0);
            }
            17 => {
                state.sample.pitch_semitones =
                    (state.sample.pitch_semitones + delta as f32 * 0.5).clamp(-24.0, 24.0);
            }
            18 => {
                state.sample.root_midi =
                    ((state.sample.root_midi as i32 + delta).clamp(0, 127)) as u8;
            }
            19 => {
                state.sample.attack =
                    (state.sample.attack + delta as f32 * 0.002).clamp(0.0005, 1.5);
            }
            20 => {
                state.sample.release =
                    (state.sample.release + delta as f32 * 0.005).clamp(0.005, 3.0);
            }
            21 => {
                if delta < 0 {
                    state.sample.play_enabled = false;
                } else if delta > 0 {
                    state.sample.play_enabled = true;
                }
            }
            _ => {}
        },
        SettingsPage::Visuals => match ui.settings_cursor {
            0 => {
                state.ui.visual_mode = cycle_visual_mode(state.ui.visual_mode, delta);
                // Clamp cursor to new row count
                let max_row = settings_row_count(SettingsPage::Visuals, state.ui.visual_mode).saturating_sub(1);
                ui.settings_cursor = ui.settings_cursor.min(max_row);
            }
            1 => match state.ui.visual_mode {
                VisualMode::Scope => state.ui.scope_show_drums = !state.ui.scope_show_drums,
                VisualMode::Donut => {
                    state.ui.donut_kick_swell =
                        (state.ui.donut_kick_swell + delta as f32 * 0.04).clamp(0.0, 1.8);
                }
                VisualMode::Cube => {
                    state.ui.cube_kick_punch =
                        (state.ui.cube_kick_punch + delta as f32 * 0.04).clamp(0.0, 2.0);
                }
                VisualMode::Camera => {
                    state.ui.visual_fx = cycle_visual_fx(state.ui.visual_fx, delta);
                }
                _ => {}
            },
            2 => match state.ui.visual_mode {
                VisualMode::Cube => {
                    state.ui.cube_hat_rewind =
                        (state.ui.cube_hat_rewind + delta as f32 * 0.04).clamp(0.0, 1.5);
                }
                VisualMode::Camera => {
                    state.ui.visual_fx_depth =
                        (state.ui.visual_fx_depth + delta as f32 * 0.05).clamp(0.0, 1.0);
                }
                _ => {}
            },
            _ => {}
        },
        SettingsPage::Project => {
            if ui.settings_cursor == 0 && !state.project.available.is_empty() {
                ui.project_index = ((ui.project_index as i32 + delta)
                    .rem_euclid(state.project.available.len() as i32))
                    as usize;
            }
        }
        SettingsPage::SoundDevice => match ui.settings_cursor {
            0 => {
                let devices = state.audio.output_devices.clone();
                cycle_audio_selection(&mut state.audio.output, &devices, delta);
            }
            1 => {
                let devices = state.audio.input_devices.clone();
                cycle_audio_selection(&mut state.audio.input, &devices, delta)
            }
            _ => {}
        },
        SettingsPage::Midi => match ui.settings_cursor {
            // Device settings
            1 => {
                if !state.midi.devices.is_empty() {
                    let current = state
                        .midi
                        .devices
                        .iter()
                        .position(|d| Some(d.name.as_str()) == state.midi.device_name.as_deref())
                        .unwrap_or(0);
                    let next = ((current as i32 + delta)
                        .rem_euclid(state.midi.devices.len() as i32))
                        as usize;
                    state.midi.device_name = Some(state.midi.devices[next].name.clone());
                }
            }
            3 => {
                state.midi.channel = match state.midi.channel {
                    MidiChannel::All if delta > 0 => MidiChannel::Index(0),
                    MidiChannel::All => MidiChannel::All,
                    MidiChannel::Index(0) if delta < 0 => MidiChannel::All,
                    MidiChannel::Index(ch) => {
                        MidiChannel::Index((ch as i32 + delta).clamp(0, 15) as u8)
                    }
                }
            }
            // Note remap settings
            6 => {
                state.midi.note_edit_in =
                    ((state.midi.note_edit_in as i32 + delta).clamp(0, 127)) as u8
            }
            8 => {
                state.midi.note_edit_out =
                    ((state.midi.note_edit_out as i32 + delta).clamp(0, 127)) as u8
            }
            // Binding settings
            11 => {
                state.midi.learn_target_index = ((state.midi.learn_target_index as i32 + delta)
                    .rem_euclid(MidiBindingTarget::ALL.len() as i32))
                    as usize;
            }
            _ => {}
        },
    }
    Ok(())
}

fn activate_setting(runtime: &mut Runtime, ui: &mut UiLocalState) -> Result<()> {
    let page = runtime.state.lock().ui.settings_page;
    match page {
        SettingsPage::Main => match ui.settings_cursor {
            11 => {
                let recording = runtime.state.lock().sample.input_recording;
                if recording {
                    runtime.end_sample_input_record();
                } else if let Err(e) = runtime.begin_sample_input_record() {
                    runtime.state.lock().audio.status = format!("Sample in: {e}");
                }
            }
            12 => {
                runtime.state.lock().sample.clear_buffer();
            }
            _ => {}
        },
        SettingsPage::Project => match ui.settings_cursor {
            1 => {
                let target = {
                    let state = runtime.state.lock();
                    state
                        .project
                        .available
                        .get(ui.project_index)
                        .cloned()
                        .unwrap_or(ProjectTarget {
                            name: "demo.mush".to_string(),
                        })
                };
                let loaded_name = target.name.clone();
                match runtime.load_project(&loaded_name) {
                    Ok(()) => {
                        let mut st = runtime.state.lock();
                        st.project.status = format!("loaded {loaded_name}");
                        if let Some(pos) = st
                            .project
                            .available
                            .iter()
                            .position(|p| p.name == loaded_name)
                        {
                            ui.project_index = pos;
                        } else {
                            ui.project_index = 0;
                        }
                    }
                    Err(e) => {
                        let mut state = runtime.state.lock();
                        state.project.status = format!("load error: {e}");
                    }
                }
            }
            2 => {
                let name_opt = {
                    let state = runtime.state.lock();
                    state
                        .project
                        .available
                        .get(ui.project_index)
                        .map(|p| p.name.clone())
                };
                let Some(name) = name_opt else {
                    let mut state = runtime.state.lock();
                    state.project.status =
                        "No project file selected (←→ pick one, or Save as new).".to_string();
                    return Ok(());
                };
                match runtime.save_project(&name) {
                    Ok(_) => {
                        let mut state = runtime.state.lock();
                        state.project.available =
                            mush_core::project_io::list_projects(runtime.base_dir()).unwrap_or_default();
                        if let Some(pos) = state.project.available.iter().position(|p| p.name == name) {
                            ui.project_index = pos;
                        } else {
                            ui.project_index = ui.project_index.min(
                                state.project.available.len().saturating_sub(1),
                            );
                        }
                        state.project.status = format!("wrote {name}");
                    }
                    Err(e) => {
                        let mut state = runtime.state.lock();
                        state.project.status = format!("save error: {e}");
                    }
                }
            }
            3 => {
                let name = match mush_core::project_io::next_free_numbered_project_name(
                    runtime.base_dir(),
                ) {
                    Ok(n) => n,
                    Err(e) => {
                        let mut state = runtime.state.lock();
                        state.project.status = format!("project list: {e}");
                        return Ok(());
                    }
                };
                match runtime.save_project(&name) {
                    Ok(_) => {
                        let mut state = runtime.state.lock();
                        state.project.available =
                            mush_core::project_io::list_projects(runtime.base_dir()).unwrap_or_default();
                        if let Some(pos) = state.project.available.iter().position(|p| p.name == name) {
                            ui.project_index = pos;
                        } else {
                            ui.project_index = ui.project_index.min(
                                state.project.available.len().saturating_sub(1),
                            );
                        }
                        state.project.status = format!("saved new {name}");
                    }
                    Err(e) => {
                        let mut state = runtime.state.lock();
                        state.project.status = format!("save error: {e}");
                    }
                }
            }
            _ => {}
        },
        SettingsPage::SoundDevice => match ui.settings_cursor {
            0 | 1 => {
                // Apply device selection
                match runtime.restart_audio() {
                    Ok(()) => {}
                    Err(e) => {
                        runtime.state.lock().audio.status = format!("Error: {e}");
                    }
                }
            }
            2 => {
                // Refresh device list
                runtime.refresh_audio_devices();
            }
            _ => {}
        }
        SettingsPage::Midi => match ui.settings_cursor {
            // Device settings
            0 => {
                let enabled = !runtime.state.lock().midi.enabled;
                runtime.set_midi_enabled(enabled)?;
            }
            2 => {
                runtime.refresh_midi_devices();
            }
            4 => {
                let mut state = runtime.state.lock();
                state.midi.note_input = !state.midi.note_input;
            }
            5 => {
                let mut state = runtime.state.lock();
                state.midi.pad_input = !state.midi.pad_input;
            }
            // Note remap settings
            7 => {
                let mut state = runtime.state.lock();
                state.midi.learn_mode =
                    if matches!(state.midi.learn_mode, MidiLearnMode::NoteSource) {
                        MidiLearnMode::Off
                    } else {
                        MidiLearnMode::NoteSource
                    };
            }
            9 => {
                let mut state = runtime.state.lock();
                let src = state.midi.note_edit_in;
                let dst = state.midi.note_edit_out;
                state.midi.note_map.insert(src, dst);
            }
            10 => {
                let mut state = runtime.state.lock();
                let src = state.midi.note_edit_in;
                state.midi.note_map.remove(&src);
            }
            // Binding settings
            12 => {
                let mut state = runtime.state.lock();
                state.midi.learn_mode =
                    if matches!(state.midi.learn_mode, MidiLearnMode::Bind) {
                        MidiLearnMode::Off
                    } else {
                        MidiLearnMode::Bind
                    };
            }
            13 => {
                let mut state = runtime.state.lock();
                let target = state.midi.selected_target();
                state.midi.bindings.remove(&target);
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn cycle_visual_fx(style: VisualFx, delta: i32) -> VisualFx {
    let all = [
        VisualFx::Off,
        VisualFx::KickFlash,
        VisualFx::SynthGlow,
        VisualFx::DrumPunch,
        VisualFx::BassScan,
        VisualFx::EdgePulse,
        VisualFx::GlitchShift,
        VisualFx::GatePoster,
        VisualFx::FireStorm,
        VisualFx::IcePulse,
        VisualFx::ChromaSplit,
        VisualFx::MatrixBeat,
        VisualFx::WaveRipple,
        VisualFx::BeatStrobe,
        VisualFx::HatSparkle,
        VisualFx::SnareBurst,
        VisualFx::BassWobble,
        VisualFx::LfoSweep,
        VisualFx::EnvelopeFade,
        VisualFx::DrumGrid,
        VisualFx::FreqShift,
        VisualFx::ComboReact,
    ];
    let idx = all.iter().position(|item| *item == style).unwrap_or(0);
    all[(idx as i32 + delta).rem_euclid(all.len() as i32) as usize]
}

fn cycle_theme(theme: Theme, delta: i32) -> Theme {
    let all = [Theme::Magenta, Theme::Mint, Theme::Amber];
    let idx = all.iter().position(|item| *item == theme).unwrap_or(0);
    all[(idx as i32 + delta).rem_euclid(all.len() as i32) as usize]
}

fn cycle_visual_mode(mode: VisualMode, delta: i32) -> VisualMode {
    let all = VisualMode::ALL;
    let idx = all.iter().position(|item| *item == mode).unwrap_or(0);
    all[(idx as i32 + delta).rem_euclid(all.len() as i32) as usize]
}

fn cycle_audio_selection(
    selection: &mut AudioDeviceSelection,
    devices: &[mush_core::state::audio::AudioDeviceInfo],
    delta: i32,
) {
    if devices.is_empty() {
        *selection = AudioDeviceSelection::DefaultSystem;
        return;
    }
    // Ring: 0 = Default OS, 1..=n = devices[0..n-1]. (Do not use device index 0 for both Default and
    // first device — that made ← work but → never advance past the first named output/input.)
    let n = devices.len() as i32;
    let current = match selection {
        AudioDeviceSelection::DefaultSystem => 0i32,
        AudioDeviceSelection::Named(name) => {
            let idx = devices.iter().position(|d| &d.name == name).unwrap_or(0) as i32;
            idx + 1
        }
    };
    let next = (current + delta).rem_euclid(n + 1);
    if next == 0 {
        *selection = AudioDeviceSelection::DefaultSystem;
    } else {
        *selection = AudioDeviceSelection::Named(devices[(next - 1) as usize].name.clone());
    }
}

fn format_audio_selection(selection: &AudioDeviceSelection) -> String {
    match selection {
        AudioDeviceSelection::DefaultSystem => "Default OS".to_string(),
        AudioDeviceSelection::Named(name) => name.clone(),
    }
}

fn short_label(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

fn bar(value: f32, width: usize) -> String {
    let width = width.max(1);
    let filled = ((value.clamp(0.0, 1.0) * width as f32).round() as usize).min(width);
    format!("{}{}", "▓".repeat(filled), "░".repeat(width - filled))
}

/// `[----S------E--]` ruler: `trim_start` / `trim_end` are fractions of buffer (same as engine).
fn sample_trim_ruler(trim_start: f32, trim_end: f32, bar_inner: usize) -> String {
    let w = bar_inner.max(6);
    let mut bytes = vec![b'-'; w];
    let max_ix = w.saturating_sub(1).max(1);
    let s_ix = (trim_start.clamp(0.0, 0.95) * max_ix as f32).round() as usize;
    let s_ix = s_ix.min(max_ix);
    let e_ix = ((1.0 - trim_end.clamp(0.0, 0.95)) * max_ix as f32)
        .round()
        .clamp(s_ix as f32, max_ix as f32) as usize;
    if s_ix >= e_ix {
        bytes[s_ix] = b'|';
    } else {
        bytes[s_ix] = b'S';
        bytes[e_ix] = b'E';
    }
    let body: String = bytes.iter().map(|&b| b as char).collect();
    format!("[{body}]")
}

fn render_scope_braille(samples: &[f32], width: usize, height: usize) -> Vec<String> {
    const BRAILLE_BASE: u32 = 0x2800;
    const DOTS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    if samples.is_empty() || width == 0 || height == 0 {
        return vec![" ".repeat(width.max(1)); height.max(1)];
    }

    let dw = width * 2;
    let dh = height * 4;
    let mut rows = Vec::with_capacity(dw);
    for dx in 0..dw {
        let idx = dx * samples.len() / dw;
        let sig = samples[idx.min(samples.len() - 1)].clamp(-1.0, 1.0);
        let row = (((1.0 - sig) * 0.5) * (dh.saturating_sub(1) as f32)).round() as usize;
        rows.push(row.min(dh.saturating_sub(1)));
    }

    let mut grid = vec![vec![0u8; width]; height];
    let plot = |dx: usize, dy: usize, grid: &mut Vec<Vec<u8>>| {
        let cx = dx / 2;
        let cy = dy / 4;
        let sx = dx % 2;
        let sy = dy % 4;
        if cx < width && cy < height {
            grid[cy][cx] |= DOTS[sx][sy];
        }
    };

    for dx in 0..dw {
        let y0 = rows[dx];
        let y1 = if dx + 1 < dw { rows[dx + 1] } else { y0 };
        let (lo, hi) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        for dy in lo..=hi {
            plot(dx, dy, &mut grid);
        }
    }

    let zy = dh / 2;
    let zcy = zy / 4;
    let zsy = zy % 4;
    if zcy < height {
        for cx in 0..width {
            if grid[zcy][cx] == 0 {
                grid[zcy][cx] |= DOTS[0][zsy];
            }
        }
    }

    grid.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cell| char::from_u32(BRAILLE_BASE | cell as u32).unwrap_or(' '))
                .collect()
        })
        .collect()
}
