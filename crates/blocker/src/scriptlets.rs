//! Built-in uBlock-compatible scriptlet templates (`##+js(...)`) registered with
//! Brave's `adblock::Engine`.

use adblock::resources::{PermissionMask, Resource, ResourceType};

/// Returns the core scriptlet resources (`json-prune`, `set-constant`, `yt-ad-skip`,
/// `no-setTimeout-if`) used by uBlock Filters rules.
pub fn builtin_scriptlet_resources() -> Vec<Resource> {
    vec![
        template_resource(
            "json-prune.js",
            &["json-prune", "ubo-json-prune.js"],
            JSON_PRUNE_JS,
        ),
        template_resource(
            "set-constant.js",
            &["set-constant", "set", "ubo-set-constant.js"],
            SET_CONSTANT_JS,
        ),
        template_resource("yt-ad-skip.js", &["yt-ad-skip"], YT_AD_SKIP_JS),
        template_resource(
            "no-setTimeout-if.js",
            &["no-setTimeout-if", "nostif", "ubo-no-setTimeout-if.js"],
            NO_SETTIMEOUT_IF_JS,
        ),
    ]
}

fn template_resource(name: &str, aliases: &[&str], js: &str) -> Resource {
    Resource {
        name: name.to_string(),
        aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
        kind: ResourceType::Template,
        content: base64_encode(js.as_bytes()),
        dependencies: Vec::new(),
        permission: PermissionMask::default(),
    }
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

const JSON_PRUNE_JS: &str = r#"(function(){
  const raw = '{{1}}';
  if (!raw || raw === '{{1}}') return;
  const paths = raw.split(/\s+/).filter(Boolean).map(p => p.split('.'));
  function pruneObj(obj) {
    if (!obj || typeof obj !== 'object') return obj;
    for (const parts of paths) {
      let cur = obj;
      for (let i = 0; i < parts.length - 1; i++) {
        if (!cur || typeof cur !== 'object') { cur = null; break; }
        cur = cur[parts[i]];
      }
      if (cur && typeof cur === 'object') {
        const last = parts[parts.length - 1];
        if (last in cur) {
          try { delete cur[last]; } catch(_) {}
        }
      }
    }
    return obj;
  }
  const origParse = JSON.parse;
  JSON.parse = function() {
    const res = origParse.apply(this, arguments);
    return pruneObj(res);
  };
  if (typeof Response !== 'undefined' && Response.prototype && Response.prototype.json) {
    const origJson = Response.prototype.json;
    Response.prototype.json = function() {
      return origJson.apply(this, arguments).then(pruneObj);
    };
  }
})();"#;

const SET_CONSTANT_JS: &str = r#"(function(){
  const chain = '{{1}}';
  const rawVal = '{{2}}';
  if (!chain || chain === '{{1}}') return;
  const val = rawVal === 'undefined' ? undefined : rawVal === 'false' ? false : rawVal === 'true' ? true : rawVal === 'null' ? null : rawVal;
  const parts = chain.split('.');
  let cur = window;
  for (let i = 0; i < parts.length - 1; i++) {
    if (!cur || typeof cur !== 'object') return;
    cur = cur[parts[i]];
  }
  if (cur && typeof cur === 'object') {
    const prop = parts[parts.length - 1];
    try {
      Object.defineProperty(cur, prop, {
        configurable: true,
        get: function() { return val; },
        set: function() {}
      });
    } catch(_) {}
  }
})();"#;

const YT_AD_SKIP_JS: &str = r#"(function(){
  if (window.__seuYtAdSkipInstalled) return;
  window.__seuYtAdSkipInstalled = true;
  function tick() {
    try {
      const player = document.querySelector('.html5-video-player');
      if (!player) return;
      const isAd = player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting');
      if (isAd) {
        const video = player.querySelector('video');
        if (video) {
          video.muted = true;
          if (Number.isFinite(video.duration) && video.duration > 0) {
            video.currentTime = video.duration;
          }
        }
        const skipBtn = document.querySelector('.ytp-skip-ad-button, .ytp-ad-skip-button, .ytp-ad-skip-button-modern');
        if (skipBtn) skipBtn.click();
      }
      const dismiss = document.querySelector('ytd-enforcement-message-view-model #dismiss-button, tp-yt-paper-dialog ytd-mealbar-promo-renderer #dismiss-button');
      if (dismiss) dismiss.click();
    } catch(_) {}
  }
  setInterval(tick, 300);
})();"#;

const NO_SETTIMEOUT_IF_JS: &str = r#"(function(){
  const needle = '{{1}}';
  if (!needle || needle === '{{1}}') return;
  const orig = window.setTimeout;
  window.setTimeout = function(fn, delay) {
    if (fn && String(fn).includes(needle)) return 0;
    return orig.apply(this, arguments);
  };
})();"#;
