use serde::{Deserialize, Serialize};

fn default_play_gain() -> f32 { 0.7 }

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoopSnapshot {
    pub length: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoopState {
    pub length: usize,
    pub write_pos: usize,
    pub read_pos: usize,
    pub recording: bool,
    pub playing: bool,
    pub overdub: bool,
    pub has_audio: bool,
    #[serde(default = "default_play_gain")]
    pub play_gain: f32,
    pub trim_start: f32,      // 0.0-1.0, fraction of loop to skip at start
    pub trim_end: f32,        // 0.0-1.0, fraction of loop to skip at end
    pub playback_speed: f32,  // 0.5 = half speed, 2.0 = double speed
    pub undo_stack: Vec<LoopSnapshot>,
    #[serde(skip)]
    pub clear_requested: bool,  // Signal to audio side to clear buffer
}

impl LoopState {
    pub fn clear(&mut self) {
        self.length = 0;
        self.write_pos = 0;
        self.read_pos = 0;
        self.recording = false;
        self.playing = false;
        self.overdub = false;
        self.has_audio = false;
        self.play_gain = 0.0;
        self.undo_stack.clear();
        self.clear_requested = true;  // Signal audio side to clear buffer
    }

    pub fn begin_replace(&mut self) {
        self.clear();
        self.recording = true;
    }

    /// Maximum undo snapshots to retain (~1.5MB each, so 10 = ~15MB max)
    const MAX_UNDO_SNAPSHOTS: usize = 10;

    pub fn begin_overdub(&mut self) {
        if self.has_audio {
            // Cap undo stack to prevent unbounded memory growth
            if self.undo_stack.len() >= Self::MAX_UNDO_SNAPSHOTS {
                self.undo_stack.remove(0);  // Drop oldest snapshot
            }
            self.undo_stack.push(self.snapshot());
            self.recording = true;
            self.playing = true;
            self.overdub = true;
            self.write_pos = self.read_pos;
        }
    }

    pub fn stop_recording(&mut self) {
        self.recording = false;
        self.overdub = false;
        if self.length > 0 {
            self.playing = true;
            self.play_gain = 0.7;
        }
    }

    pub fn undo_last(&mut self) -> bool {
        if self.recording {
            return false;
        }

        if let Some(snapshot) = self.undo_stack.pop() {
            self.length = snapshot.length;
            self.has_audio = snapshot.length > 0;
            self.playing = self.has_audio;
            self.recording = false;
            self.overdub = false;
            self.read_pos = 0;
            self.write_pos = 0;
            if self.play_gain == 0.0 && self.has_audio {
                self.play_gain = 0.7;
            }
            true
        } else {
            false
        }
    }

    pub fn snapshot(&self) -> LoopSnapshot {
        LoopSnapshot {
            length: self.length,
        }
    }
}

impl Default for LoopState {
    fn default() -> Self {
        Self {
            length: 0,
            write_pos: 0,
            read_pos: 0,
            recording: false,
            playing: false,
            overdub: false,
            has_audio: false,
            play_gain: 0.7,
            trim_start: 0.0,
            trim_end: 0.0,
            playback_speed: 1.0,
            undo_stack: Vec::new(),
            clear_requested: false,
        }
    }
}
