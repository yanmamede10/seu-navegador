//! Platform-agnostic Game Mode configuration, status, and debounced
//! auto-detection state machine (SPEC §9).
//!
//! Uses injectable timestamps (`now_ms: u64`) so debounce, Borderless exclusion
//! of the browser's own process, `.exe` matching, and minimized triggers are
//! 100% deterministically unit-testable without Win32 or wall-clock sleeps.

use serde::{Deserialize, Serialize};

pub const DEFAULT_GAME_DEBOUNCE_SECS: u64 = 6;
pub const DEFAULT_MINIMIZED_TRIGGER_SECS: u64 = 30;

pub fn default_game_executables() -> Vec<String> {
    vec![
        "cs2.exe".into(),
        "valorant-win64-shipping.exe".into(),
        "eldenring.exe".into(),
        "cyberpunk2077.exe".into(),
        "gta5.exe".into(),
        "dota2.exe".into(),
        "league of legends.exe".into(),
        "fortniteclient-win64-shipping.exe".into(),
        "balatro.exe".into(),
        "minecraft.exe".into(),
        "javaw.exe".into(),
    ]
}

fn default_true() -> bool {
    true
}

fn default_debounce_secs() -> u64 {
    DEFAULT_GAME_DEBOUNCE_SECS
}

fn default_minimized_trigger_secs() -> u64 {
    DEFAULT_MINIMIZED_TRIGGER_SECS
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameModeConfig {
    /// Automatically detect fullscreen games / configured `.exe` / minimized browser.
    #[serde(default = "default_true")]
    pub auto_detect: bool,
    /// Continuous seconds a fullscreen/game signal must hold before activating (5–10s recommended).
    #[serde(default = "default_debounce_secs")]
    pub debounce_secs: u64,
    /// Seconds all browser windows stay minimized before entering Game Mode (`0` disables).
    #[serde(default = "default_minimized_trigger_secs")]
    pub minimized_trigger_secs: u64,
    /// Case-insensitive executable basenames (e.g. `eldenring.exe`) that trigger Game Mode.
    #[serde(default = "default_game_executables")]
    pub game_executables: Vec<String>,
}

impl Default for GameModeConfig {
    fn default() -> Self {
        Self {
            auto_detect: true,
            debounce_secs: DEFAULT_GAME_DEBOUNCE_SECS,
            minimized_trigger_secs: DEFAULT_MINIMIZED_TRIGGER_SECS,
            game_executables: default_game_executables(),
        }
    }
}

impl GameModeConfig {
    pub fn sanitized(mut self) -> Self {
        self.debounce_secs = self.debounce_secs.clamp(3, 30);
        if self.minimized_trigger_secs > 0 {
            self.minimized_trigger_secs = self.minimized_trigger_secs.clamp(5, 600);
        }
        let mut seen = Vec::new();
        for raw in self.game_executables {
            let clean = raw
                .trim()
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if !clean.is_empty() && !seen.contains(&clean) {
                seen.push(clean);
            }
        }
        self.game_executables = seen;
        self
    }

    pub fn matches_executable(&self, exe_name: &str) -> bool {
        let norm = exe_name
            .trim()
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if norm.is_empty() {
            return false;
        }
        self.game_executables
            .iter()
            .any(|configured| configured.eq_ignore_ascii_case(&norm))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameModeTrigger {
    Manual,
    D3dFullscreen,
    BorderlessFullscreen,
    GameExe,
    Minimized,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GameModeStatus {
    pub active: bool,
    pub trigger: Option<GameModeTrigger>,
    pub detected_process: Option<String>,
    pub tabs_slept: usize,
    pub ram_before_mb: u32,
    pub ram_saved_mb: u32,
    /// Set when exiting Game Mode so UI displays "X MB devolvidos durante o jogo".
    pub last_exit_saved_mb: u32,
    pub last_exit_at_ms: u64,
}

/// Snapshot sampled from Win32 (or tests) on each monitor tick.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GameProbeSample {
    pub now_ms: u64,
    pub d3d_fullscreen: bool,
    pub borderless_fullscreen: bool,
    /// True when the foreground window belongs to `seu-navegador.exe` (e.g., YouTube fullscreen).
    pub foreground_is_own_browser: bool,
    pub foreground_exe: Option<String>,
    pub all_browser_windows_minimized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameModeTransition {
    None,
    Enter {
        trigger: GameModeTrigger,
        process: Option<String>,
    },
    Exit,
}

#[derive(Debug, Default)]
pub struct GameModeDetector {
    pending_enter_since_ms: Option<u64>,
    pending_minimized_since_ms: Option<u64>,
    pending_exit_since_ms: Option<u64>,
}

impl GameModeDetector {
    pub fn reset(&mut self) {
        self.pending_enter_since_ms = None;
        self.pending_minimized_since_ms = None;
        self.pending_exit_since_ms = None;
    }

    /// Evaluates one OS probe sample against the active config and current status.
    pub fn evaluate(
        &mut self,
        sample: &GameProbeSample,
        config: &GameModeConfig,
        status: &GameModeStatus,
    ) -> GameModeTransition {
        if !config.auto_detect {
            self.reset();
            return GameModeTransition::None;
        }

        // Never auto-exit a session that the user manually toggled ON.
        if status.active && status.trigger == Some(GameModeTrigger::Manual) {
            self.reset();
            return GameModeTransition::None;
        }

        let fg_signal: Option<(GameModeTrigger, Option<String>)> =
            if sample.foreground_is_own_browser {
                // Critical SPEC §9 rule: YouTube/video fullscreen inside Seu Navegador
                // must NEVER trigger Game Mode.
                None
            } else if let Some(ref exe) = sample.foreground_exe {
                if config.matches_executable(exe) {
                    Some((GameModeTrigger::GameExe, Some(exe.clone())))
                } else if sample.d3d_fullscreen {
                    Some((GameModeTrigger::D3dFullscreen, Some(exe.clone())))
                } else if sample.borderless_fullscreen {
                    Some((GameModeTrigger::BorderlessFullscreen, Some(exe.clone())))
                } else {
                    None
                }
            } else if sample.d3d_fullscreen {
                Some((GameModeTrigger::D3dFullscreen, None))
            } else if sample.borderless_fullscreen {
                Some((GameModeTrigger::BorderlessFullscreen, None))
            } else {
                None
            };

        let debounce_ms = config.debounce_secs.clamp(3, 30) * 1000;
        let min_ms = config.minimized_trigger_secs * 1000;

        if !status.active {
            self.pending_exit_since_ms = None;

            if let Some((trigger, process)) = fg_signal {
                self.pending_minimized_since_ms = None;
                let since = *self.pending_enter_since_ms.get_or_insert(sample.now_ms);
                if sample.now_ms.saturating_sub(since) >= debounce_ms {
                    self.pending_enter_since_ms = None;
                    return GameModeTransition::Enter { trigger, process };
                }
                return GameModeTransition::None;
            }
            self.pending_enter_since_ms = None;

            if min_ms > 0 && sample.all_browser_windows_minimized {
                let since = *self.pending_minimized_since_ms.get_or_insert(sample.now_ms);
                if sample.now_ms.saturating_sub(since) >= min_ms {
                    self.pending_minimized_since_ms = None;
                    return GameModeTransition::Enter {
                        trigger: GameModeTrigger::Minimized,
                        process: None,
                    };
                }
            } else {
                self.pending_minimized_since_ms = None;
            }
            GameModeTransition::None
        } else {
            // Currently active via auto-detection. Exit only when both foreground game
            // and minimized condition are gone for the exit debounce period.
            let still_triggered =
                fg_signal.is_some() || (min_ms > 0 && sample.all_browser_windows_minimized);
            if still_triggered {
                self.pending_exit_since_ms = None;
                return GameModeTransition::None;
            }
            let since = *self.pending_exit_since_ms.get_or_insert(sample.now_ms);
            if sample.now_ms.saturating_sub(since) >= debounce_ms.min(5000) {
                self.pending_exit_since_ms = None;
                return GameModeTransition::Exit;
            }
            GameModeTransition::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_triggers_on_own_browser_fullscreen_youtube() {
        let mut det = GameModeDetector::default();
        let cfg = GameModeConfig {
            debounce_secs: 5,
            ..GameModeConfig::default()
        };
        let status = GameModeStatus::default();

        for t in [0, 3_000, 6_000, 12_000] {
            let sample = GameProbeSample {
                now_ms: t,
                d3d_fullscreen: true,
                borderless_fullscreen: true,
                foreground_is_own_browser: true,
                foreground_exe: Some("seu-navegador.exe".into()),
                all_browser_windows_minimized: false,
            };
            assert_eq!(
                det.evaluate(&sample, &cfg, &status),
                GameModeTransition::None
            );
        }
    }

    #[test]
    fn triggers_borderless_and_exe_after_debounce_and_exits_cleanly() {
        let mut det = GameModeDetector::default();
        let cfg = GameModeConfig {
            debounce_secs: 6,
            ..GameModeConfig::default()
        };
        let mut status = GameModeStatus::default();

        let mk = |now_ms: u64, exe: Option<&str>, borderless: bool| GameProbeSample {
            now_ms,
            d3d_fullscreen: false,
            borderless_fullscreen: borderless,
            foreground_is_own_browser: false,
            foreground_exe: exe.map(String::from),
            all_browser_windows_minimized: false,
        };

        // At t=0 and t=4s: still debouncing.
        assert_eq!(
            det.evaluate(&mk(1_000, Some("eldenring.exe"), true), &cfg, &status),
            GameModeTransition::None
        );
        assert_eq!(
            det.evaluate(&mk(5_000, Some("eldenring.exe"), true), &cfg, &status),
            GameModeTransition::None
        );
        // At t=7s (>= 6s): enters Game Mode!
        assert_eq!(
            det.evaluate(&mk(7_000, Some("eldenring.exe"), true), &cfg, &status),
            GameModeTransition::Enter {
                trigger: GameModeTrigger::GameExe,
                process: Some("eldenring.exe".into()),
            }
        );

        status.active = true;
        status.trigger = Some(GameModeTrigger::GameExe);

        // Leaves game: debounces exit then fires Exit.
        assert_eq!(
            det.evaluate(&mk(10_000, Some("explorer.exe"), false), &cfg, &status),
            GameModeTransition::None
        );
        assert_eq!(
            det.evaluate(&mk(16_000, Some("explorer.exe"), false), &cfg, &status),
            GameModeTransition::Exit
        );
    }

    #[test]
    fn manual_mode_is_not_auto_exited() {
        let mut det = GameModeDetector::default();
        let cfg = GameModeConfig::default();
        let status = GameModeStatus {
            active: true,
            trigger: Some(GameModeTrigger::Manual),
            ..GameModeStatus::default()
        };
        let sample = GameProbeSample {
            now_ms: 50_000,
            ..GameProbeSample::default()
        };
        assert_eq!(
            det.evaluate(&sample, &cfg, &status),
            GameModeTransition::None
        );
    }

    #[test]
    fn triggers_when_minimized_longer_than_threshold() {
        let mut det = GameModeDetector::default();
        let cfg = GameModeConfig {
            minimized_trigger_secs: 20,
            ..GameModeConfig::default()
        };
        let status = GameModeStatus::default();
        let s1 = GameProbeSample {
            now_ms: 1_000,
            all_browser_windows_minimized: true,
            ..GameProbeSample::default()
        };
        let s2 = GameProbeSample {
            now_ms: 21_500,
            all_browser_windows_minimized: true,
            ..GameProbeSample::default()
        };
        assert_eq!(det.evaluate(&s1, &cfg, &status), GameModeTransition::None);
        assert_eq!(
            det.evaluate(&s2, &cfg, &status),
            GameModeTransition::Enter {
                trigger: GameModeTrigger::Minimized,
                process: None,
            }
        );
    }
}
