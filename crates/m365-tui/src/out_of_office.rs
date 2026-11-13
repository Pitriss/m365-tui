//! Out of Office editor helpers.
//!
//! Scheduled OOO values are edited as segmented local wall-clock fields. Each
//! segment is validated when focus leaves it; the complete local timestamp is
//! validated when leaving the minute field so DST gaps/overlaps are rejected.
//! Graph is sent UTC instants, avoiding Windows/IANA time-zone ambiguity.

use std::path::{Path, PathBuf};

use chrono::{
  format::{Item, Locale, StrftimeItems}, DateTime, Duration, Local, LocalResult, NaiveDate,
  NaiveDateTime, NaiveTime, TimeZone, Utc,
};
use m365_core::models::{
  AutomaticRepliesSetting, DateTimeTimeZone, MailboxSettingsSnapshot,
};

use crate::content;
use crate::editor::TextInput;

pub const OUTLOOK_OOO_URL: &str =
  "https://outlook.office.com/mail/options/accounts-category/automaticReply";

const OOO_TEMPLATE_STATE_FILE: &str = "ooo-template.json";
const OOO_TEMPLATE_STATE_VERSION: u64 = 1;
const TEMPLATE_FUZZY_SLOT_MAX_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateRestore {
  None,
  Current,
  Previous,
  Structure,
}

impl TemplateRestore {
  pub fn status_message(self) -> Option<&'static str> {
    match self {
      Self::None | Self::Current => None,
      Self::Previous => Some(
        "OOO template restored; stored message still contains previous template values",
      ),
      Self::Structure => Some(
        "OOO template restored from matching message structure; review dates before saving",
      ),
    }
  }
}

#[derive(Debug, Clone)]
pub struct TemplateState {
  template: String,
  locale: String,
  markdown: bool,
  last_rendered: String,
}

pub struct PreparedOutOfOffice {
  pub setting: AutomaticRepliesSetting,
  pub template_state: Option<TemplateState>,
}

#[derive(Debug, Clone, Copy)]
enum TemplateSlot {
  Start,
  End,
}

#[derive(Debug, Clone)]
enum TemplateSegment {
  Literal(String),
  Slot {
    slot: TemplateSlot,
    format: String,
  },
}

#[derive(Debug, Clone)]
enum MatchSegment {
  Literal(String),
  Slot,
}

struct RenderedTemplate {
  html: String,
  plain: String,
  templated: bool,
}

fn placeholder_prefix(after_open: &str) -> Option<(TemplateSlot, usize, &'static str)> {
  for (name, slot) in [("end", TemplateSlot::End), ("st", TemplateSlot::Start)] {
    if !after_open.starts_with(name) {
      continue;
    }
    let tail = &after_open[name.len()..];
    match tail.chars().next() {
      Some('}') => return Some((slot, name.len(), name)),
      Some(ch) if ch.is_whitespace() => return Some((slot, name.len(), name)),
      _ => {}
    }
  }
  None
}

fn validate_template_format(format: &str, name: &str) -> Result<(), String> {
  if format.is_empty() {
    return Err(format!("OOO template {{{name} ...}} needs a date format"));
  }
  if StrftimeItems::new(format).any(|item| matches!(item, Item::Error)) {
    return Err(format!(
      "OOO template {{{name} {format}}} contains an unsupported date format"
    ));
  }
  Ok(())
}

fn parse_template(input: &str) -> Result<Vec<TemplateSegment>, String> {
  let mut segments = Vec::new();
  let mut search = 0usize;
  let mut literal_start = 0usize;

  while let Some(relative_open) = input[search..].find('{') {
    let open = search + relative_open;
    let after_open = &input[open + 1..];
    let Some((slot, name_len, name)) = placeholder_prefix(after_open) else {
      search = open + 1;
      continue;
    };

    let Some(relative_close) = after_open.find('}') else {
      return Err(format!("OOO template {{{name} ... is missing closing }}"));
    };
    let inner = &after_open[..relative_close];
    let format = inner[name_len..].trim();
    validate_template_format(format, name)?;

    if open > literal_start {
      segments.push(TemplateSegment::Literal(input[literal_start..open].to_string()));
    }
    segments.push(TemplateSegment::Slot {
      slot,
      format: format.to_string(),
    });

    let close = open + 1 + relative_close;
    search = close + 1;
    literal_start = search;
  }

  if literal_start < input.len() {
    segments.push(TemplateSegment::Literal(input[literal_start..].to_string()));
  }

  Ok(segments)
}

fn template_has_slots(segments: &[TemplateSegment]) -> bool {
  segments
    .iter()
    .any(|segment| matches!(segment, TemplateSegment::Slot { .. }))
}

fn expand_segments(
  segments: &[TemplateSegment],
  start: &DateTime<Local>,
  end: &DateTime<Local>,
  locale: Locale,
) -> String {
  let mut out = String::new();
  for segment in segments {
    match segment {
      TemplateSegment::Literal(text) => out.push_str(text),
      TemplateSegment::Slot { slot, format } => {
        let value = match slot {
          TemplateSlot::Start => start,
          TemplateSlot::End => end,
        };
        out.push_str(&value.format_localized(format, locale).to_string());
      }
    }
  }
  out
}

fn normalize_locale_name(raw: &str) -> Option<String> {
  let raw = raw.trim();
  if raw.is_empty() {
    return None;
  }

  let (base_with_encoding, modifier) = raw
    .split_once('@')
    .map(|(base, modifier)| (base, Some(modifier)))
    .unwrap_or((raw, None));
  let base = base_with_encoding
    .split_once('.')
    .map(|(base, _)| base)
    .unwrap_or(base_with_encoding)
    .trim();

  if base.eq_ignore_ascii_case("C") || base.eq_ignore_ascii_case("POSIX") {
    return Some("POSIX".into());
  }
  if base.is_empty() {
    return None;
  }

  Some(match modifier.map(str::trim).filter(|value| !value.is_empty()) {
    Some(modifier) => format!("{base}@{modifier}"),
    None => base.to_string(),
  })
}

fn current_locale() -> (String, Locale) {
  for key in ["LC_ALL", "LC_TIME", "LANG"] {
    let Ok(raw) = std::env::var(key) else {
      continue;
    };
    let Some(name) = normalize_locale_name(&raw) else {
      continue;
    };
    if let Ok(locale) = Locale::try_from(name.as_str()) {
      return (locale.to_string(), locale);
    }
  }
  (Locale::POSIX.to_string(), Locale::POSIX)
}

fn render_template_message(
  template: &str,
  start: &DateTime<Local>,
  end: &DateTime<Local>,
  locale: Locale,
  markdown: bool,
) -> Result<RenderedTemplate, String> {
  let segments = parse_template(template)?;
  let templated = template_has_slots(&segments);
  let expanded = expand_segments(&segments, start, end, locale);
  let html = m365_core::teams_format::outgoing_body(&expanded, markdown).html_fragment();
  let plain = html_to_plain(&html);
  Ok(RenderedTemplate {
    html,
    plain,
    templated,
  })
}

fn normalize_match_text(input: &str) -> String {
  let mut out = String::with_capacity(input.len());
  let mut whitespace = false;
  for ch in input.chars() {
    if ch.is_whitespace() {
      whitespace = true;
      continue;
    }
    if whitespace && !out.is_empty() {
      out.push(' ');
    }
    whitespace = false;
    out.push(ch);
  }
  out
}

fn visible_match_segments(template: &str, markdown: bool) -> Result<Vec<MatchSegment>, String> {
  const TOKEN_PREFIX: &str = "M365OOOTEMPLATESLOT9F3C";

  if template.contains(TOKEN_PREFIX) {
    return Err("OOO template contains a reserved internal marker".into());
  }

  let segments = parse_template(template)?;
  if !template_has_slots(&segments) {
    return Ok(Vec::new());
  }

  let mut source = String::new();
  let mut tokens = Vec::new();
  for segment in segments {
    match segment {
      TemplateSegment::Literal(text) => source.push_str(&text),
      TemplateSegment::Slot { .. } => {
        let token = format!("{TOKEN_PREFIX}{}X", tokens.len());
        source.push_str(&token);
        tokens.push(token);
      }
    }
  }

  let html = m365_core::teams_format::outgoing_body(&source, markdown).html_fragment();
  let plain = html_to_plain(&html);
  let mut result = Vec::new();
  let mut cursor = 0usize;

  for token in tokens {
    let Some(relative) = plain[cursor..].find(&token) else {
      return Ok(Vec::new());
    };
    let start = cursor + relative;
    if start > cursor {
      let literal = normalize_match_text(&plain[cursor..start]);
      if !literal.is_empty() {
        result.push(MatchSegment::Literal(literal));
      }
    }
    result.push(MatchSegment::Slot);
    cursor = start + token.len();
  }

  if cursor < plain.len() {
    let literal = normalize_match_text(&plain[cursor..]);
    if !literal.is_empty() {
      result.push(MatchSegment::Literal(literal));
    }
  }

  Ok(result)
}

fn valid_fuzzy_capture(value: &str) -> bool {
  let value = value.trim();
  !value.is_empty() && value.chars().count() <= TEMPLATE_FUZZY_SLOT_MAX_CHARS
}

fn match_visible_segments(
  segments: &[MatchSegment],
  segment_index: usize,
  text: &str,
  text_index: usize,
) -> bool {
  if segment_index == segments.len() {
    return text[text_index..].trim().is_empty();
  }

  match &segments[segment_index] {
    MatchSegment::Literal(literal) => text[text_index..]
      .strip_prefix(literal)
      .is_some_and(|remaining| {
        let next_index = text.len() - remaining.len();
        match_visible_segments(segments, segment_index + 1, text, next_index)
      }),
    MatchSegment::Slot => {
      if matches!(segments.get(segment_index + 1), Some(MatchSegment::Slot)) {
        return false;
      }

      let Some(MatchSegment::Literal(next_literal)) = segments.get(segment_index + 1) else {
        return valid_fuzzy_capture(&text[text_index..]);
      };

      for (relative, _) in text[text_index..].match_indices(next_literal) {
        let next_index = text_index + relative;
        let capture = &text[text_index..next_index];
        if capture.chars().count() > TEMPLATE_FUZZY_SLOT_MAX_CHARS {
          break;
        }
        if valid_fuzzy_capture(capture)
          && match_visible_segments(segments, segment_index + 1, text, next_index)
        {
          return true;
        }
      }
      false
    }
  }
}

fn template_structure_matches(template: &str, server_plain: &str, markdown: bool) -> bool {
  let Ok(segments) = visible_match_segments(template, markdown) else {
    return false;
  };
  if segments.is_empty() {
    return false;
  }
  let server = normalize_match_text(server_plain);
  match_visible_segments(&segments, 0, &server, 0)
}

fn restore_template(
  state: &TemplateState,
  server_plain: &str,
  start: &DateTime<Local>,
  end: &DateTime<Local>,
) -> Option<(String, TemplateRestore)> {
  let locale = Locale::try_from(state.locale.as_str()).ok()?;
  let current = render_template_message(&state.template, start, end, locale, state.markdown).ok()?;
  if !current.templated {
    return None;
  }

  let server = normalize_match_text(server_plain);
  if normalize_match_text(&current.plain) == server {
    return Some((state.template.clone(), TemplateRestore::Current));
  }
  if !state.last_rendered.trim().is_empty()
    && normalize_match_text(&state.last_rendered) == server
  {
    return Some((state.template.clone(), TemplateRestore::Previous));
  }
  if template_structure_matches(&state.template, server_plain, state.markdown) {
    return Some((state.template.clone(), TemplateRestore::Structure));
  }
  None
}

fn template_state_path(token_cache_path: &Path) -> PathBuf {
  token_cache_path.with_file_name(OOO_TEMPLATE_STATE_FILE)
}

pub fn load_template_state(token_cache_path: &Path) -> std::io::Result<Option<TemplateState>> {
  let path = template_state_path(token_cache_path);
  let text = match std::fs::read_to_string(&path) {
    Ok(text) => text,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
    Err(error) => return Err(error),
  };

  let value: serde_json::Value = serde_json::from_str(&text)
    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
  let version = value.get("version").and_then(serde_json::Value::as_u64);
  if version != Some(OOO_TEMPLATE_STATE_VERSION) {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "unsupported OOO template state version",
    ));
  }

  let required_string = |key: &str| -> std::io::Result<String> {
    value
      .get(key)
      .and_then(serde_json::Value::as_str)
      .map(str::to_string)
      .ok_or_else(|| {
        std::io::Error::new(
          std::io::ErrorKind::InvalidData,
          format!("OOO template state is missing {key}"),
        )
      })
  };

  let template = required_string("template")?;
  let locale = required_string("locale")?;
  let last_rendered = required_string("last_rendered")?;
  let markdown = value
    .get("markdown")
    .and_then(serde_json::Value::as_bool)
    .ok_or_else(|| {
      std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "OOO template state is missing markdown",
      )
    })?;

  let segments = parse_template(&template)
    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
  if !template_has_slots(&segments) || Locale::try_from(locale.as_str()).is_err() {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "OOO template state contains an invalid template or locale",
    ));
  }

  Ok(Some(TemplateState {
    template,
    locale,
    markdown,
    last_rendered,
  }))
}

#[cfg(unix)]
fn write_private_template_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
  use std::io::Write;
  use std::os::unix::fs::OpenOptionsExt;

  let mut file = std::fs::OpenOptions::new()
    .create(true)
    .truncate(true)
    .write(true)
    .mode(0o600)
    .open(path)?;
  file.write_all(bytes)?;
  file.flush()
}

#[cfg(not(unix))]
fn write_private_template_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
  use std::io::Write;

  let mut file = std::fs::OpenOptions::new()
    .create(true)
    .truncate(true)
    .write(true)
    .open(path)?;
  file.write_all(bytes)?;
  file.flush()
}

pub fn persist_template_state(
  token_cache_path: &Path,
  state: Option<&TemplateState>,
) -> std::io::Result<()> {
  let path = template_state_path(token_cache_path);
  let Some(state) = state else {
    return match std::fs::remove_file(path) {
      Ok(()) => Ok(()),
      Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
      Err(error) => Err(error),
    };
  };

  let value = serde_json::json!({
    "version": OOO_TEMPLATE_STATE_VERSION,
    "template": state.template.as_str(),
    "locale": state.locale.as_str(),
    "markdown": state.markdown,
    "last_rendered": state.last_rendered.as_str(),
  });
  let bytes = serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?;
  let temp = path.with_extension(format!("json.tmp-{}", std::process::id()));
  write_private_template_file(&temp, &bytes)?;

  #[cfg(unix)]
  std::fs::rename(&temp, &path)?;

  #[cfg(not(unix))]
  {
    if path.exists() {
      std::fs::remove_file(&path)?;
    }
    std::fs::rename(&temp, &path)?;
  }

  Ok(())
}

pub const START_YEAR: usize = 0;
pub const START_MONTH: usize = 1;
pub const START_DAY: usize = 2;
pub const START_HOUR: usize = 3;
pub const START_MINUTE: usize = 4;
pub const END_YEAR: usize = 5;
pub const END_MONTH: usize = 6;
pub const END_DAY: usize = 7;
pub const END_HOUR: usize = 8;
pub const END_MINUTE: usize = 9;
pub const AUDIENCE_FIELD: usize = 10;
pub const MESSAGE_FIELD: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
  Internal,
  All,
}

impl Audience {
  pub fn label(self) -> &'static str {
    match self {
      Self::Internal => "Internal",
      Self::All => "All",
    }
  }

  pub fn toggle(&mut self) {
    *self = match self {
      Self::Internal => Self::All,
      Self::All => Self::Internal,
    };
  }
}

#[derive(Debug, Clone)]
pub struct View {
  pub settings: MailboxSettingsSnapshot,
  pub status: String,
  pub start: Option<String>,
  pub end: Option<String>,
  pub start_detail: Option<String>,
  pub end_detail: Option<String>,
  pub audience: Audience,
  pub message: String,
  pub local_zone: String,
  pub mailbox_zone: Option<String>,
  pub stored_zone: Option<String>,
  pub external_differs: bool,
  pub dst_transition: bool,
}

#[derive(Debug, Clone)]
pub struct Editor {
  pub start_year: TextInput,
  pub start_month: TextInput,
  pub start_day: TextInput,
  pub start_hour: TextInput,
  pub start_minute: TextInput,
  pub end_year: TextInput,
  pub end_month: TextInput,
  pub end_day: TextInput,
  pub end_hour: TextInput,
  pub end_minute: TextInput,
  pub audience: Audience,
  pub message: TextInput,
  pub field: usize,
  pub scroll: u16,
  pub validation_error: Option<String>,
  pub local_zone: String,
  pub mailbox_zone: Option<String>,
  pub stored_zone: Option<String>,
  pub template_restore: TemplateRestore,
  replace_on_type: bool,
  original_plain: String,
  original_internal_html: Option<String>,
}

impl Editor {
  pub const FIELD_COUNT: usize = 12;

  pub fn from_settings(settings: &MailboxSettingsSnapshot) -> Self {
    Self::from_settings_with_template(settings, None)
  }

  pub fn from_settings_with_template(
    settings: &MailboxSettingsSnapshot,
    template_state: Option<&TemplateState>,
  ) -> Self {
    let setting = &settings.automatic_replies_setting;
    let (default_start, default_end) = default_range();

    let start = setting
      .scheduled_start_date_time
      .as_ref()
      .and_then(graph_datetime_for_editor)
      .unwrap_or(default_start);
    let end = setting
      .scheduled_end_date_time
      .as_ref()
      .and_then(graph_datetime_for_editor)
      .unwrap_or(default_end);

    let original_internal_html = setting.internal_reply_message.clone();
    let original_plain = original_internal_html
      .as_deref()
      .map(html_to_plain)
      .unwrap_or_default();

    let (message, template_restore) = template_state
      .and_then(|state| restore_template(state, &original_plain, &start, &end))
      .unwrap_or_else(|| {
        (original_plain.clone(), TemplateRestore::None)
      });

    Self {
      start_year: input(start.format("%Y")),
      start_month: input(start.format("%m")),
      start_day: input(start.format("%d")),
      start_hour: input(start.format("%H")),
      start_minute: input(start.format("%M")),
      end_year: input(end.format("%Y")),
      end_month: input(end.format("%m")),
      end_day: input(end.format("%d")),
      end_hour: input(end.format("%H")),
      end_minute: input(end.format("%M")),
      audience: audience_from_setting(setting),
      message: TextInput::from(message.as_str()),
      field: START_YEAR,
      scroll: 0,
      validation_error: None,
      local_zone: local_timezone_name(),
      mailbox_zone: settings.time_zone.clone(),
      stored_zone: setting
        .scheduled_start_date_time
        .as_ref()
        .and_then(|value| value.time_zone.clone()),
      template_restore,
      replace_on_type: true,
      original_plain,
      original_internal_html,
    }
  }

  pub fn is_message_field(&self) -> bool {
    self.field == MESSAGE_FIELD
  }

  pub fn is_audience_field(&self) -> bool {
    self.field == AUDIENCE_FIELD
  }

  pub fn is_numeric_field(&self) -> bool {
    self.field <= END_MINUTE
  }

  pub fn active_mut(&mut self) -> Option<&mut TextInput> {
    self.input_mut(self.field)
  }

  pub fn mark_editing(&mut self) {
    self.replace_on_type = false;
    self.validation_error = None;
  }

  pub fn next_field(&mut self) {
    self.leave_field(1);
  }

  pub fn prev_field(&mut self) {
    self.leave_field(-1);
  }

  pub fn insert_digit(&mut self, ch: char) {
    if !ch.is_ascii_digit() || !self.is_numeric_field() {
      return;
    }

    let field = self.field;
    let max_len = numeric_width(field);
    if self.replace_on_type {
      if let Some(value) = self.input_mut(field) {
        value.clear();
      }
      self.replace_on_type = false;
    }

    let current_len = self
      .input(field)
      .map(|value| value.text().chars().count())
      .unwrap_or(0);

    if current_len >= max_len {
      self.validation_error = Some(format!(
        "{} accepts at most {max_len} digits",
        field_label(field)
      ));
      return;
    }

    if let Some(value) = self.input_mut(field) {
      value.insert(ch);
    }
    self.validation_error = None;

    let new_len = self
      .input(field)
      .map(|value| value.text().chars().count())
      .unwrap_or(0);

    // Full-width input advances automatically only when it validates.
    if new_len == max_len {
      self.leave_field(1);
    }
  }

  pub fn reject_non_digit(&mut self) {
    if self.is_numeric_field() {
      self.validation_error = Some(format!("{} accepts digits only", field_label(self.field)));
    }
  }

  pub fn paste(&mut self, text: &str) {
    if self.is_message_field() {
      self.message.insert_str(text);
      self.validation_error = None;
      return;
    }
    if !self.is_numeric_field() {
      return;
    }

    let digits: String = text.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
      self.reject_non_digit();
      return;
    }

    let max_len = numeric_width(self.field);
    let digits: String = digits.chars().take(max_len).collect();
    let field = self.field;
    if let Some(value) = self.input_mut(field) {
      *value = TextInput::from(digits.as_str());
    }
    self.replace_on_type = false;
    self.validation_error = None;

    if digits.chars().count() == max_len {
      self.leave_field(1);
    }
  }

  pub fn page_up(&mut self) {
    self.scroll = self.scroll.saturating_sub(3);
  }

  pub fn page_down(&mut self) {
    self.scroll = self.scroll.saturating_add(3).min(16);
  }

  pub fn focus_row(&self) -> u16 {
    match self.field {
      START_YEAR | START_MONTH | START_DAY => 2,
      START_HOUR | START_MINUTE => 3,
      END_YEAR | END_MONTH | END_DAY => 4,
      END_HOUR | END_MINUTE => 5,
      AUDIENCE_FIELD => 6,
      MESSAGE_FIELD => 10,
      _ => 0,
    }
  }

  pub fn start_detail(&self) -> String {
    self.start_local()
      .map(|value| local_time_detail_value(&value))
      .unwrap_or_else(|error| format!("! {error}"))
  }

  pub fn end_detail(&self) -> String {
    self.end_local()
      .map(|value| local_time_detail_value(&value))
      .unwrap_or_else(|error| format!("! {error}"))
  }

  pub fn dst_transition(&self) -> bool {
    let Ok(start) = self.start_local() else {
      return false;
    };
    let Ok(end) = self.end_local() else {
      return false;
    };
    start.offset().local_minus_utc() != end.offset().local_minus_utc()
  }

  pub fn prepare(
    &self,
    markdown: bool,
  ) -> Result<PreparedOutOfOffice, String> {
    let start = self.start_local()?;
    let end = self.end_local()?;
    if end <= start {
      return Err("OOO end must be after start".into());
    }

    let message = self.message.text();
    if message.trim().is_empty() {
      return Err("OOO message cannot be empty".into());
    }

    let (locale_name, locale) = current_locale();
    let rendered = render_template_message(&message, &start, &end, locale, markdown)?;
    let templated = rendered.templated;
    let last_rendered = rendered.plain.clone();

    let html = if !templated && message == self.original_plain {
      self.original_internal_html
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(rendered.html)
    } else {
      rendered.html
    };

    let template_state = templated.then_some(TemplateState {
      template: message,
      locale: locale_name,
      markdown,
      last_rendered,
    });

    let external = match self.audience {
      Audience::Internal => String::new(),
      Audience::All => html.clone(),
    };

    let setting = AutomaticRepliesSetting {
      status: "scheduled".into(),
      external_audience: Some(
        match self.audience {
          Audience::Internal => "none",
          Audience::All => "all",
        }
        .into(),
      ),
      scheduled_start_date_time: Some(DateTimeTimeZone {
        date_time: start
          .with_timezone(&Utc)
          .format("%Y-%m-%dT%H:%M:%S")
          .to_string(),
        time_zone: Some("UTC".into()),
      }),
      scheduled_end_date_time: Some(DateTimeTimeZone {
        date_time: end
          .with_timezone(&Utc)
          .format("%Y-%m-%dT%H:%M:%S")
          .to_string(),
        time_zone: Some("UTC".into()),
      }),
      internal_reply_message: Some(html),
      external_reply_message: Some(external),
    };

    Ok(PreparedOutOfOffice {
      setting,
      template_state,
    })
  }

  pub fn setting(&self, markdown: bool) -> Result<AutomaticRepliesSetting, String> {
    self.prepare(markdown).map(|prepared| prepared.setting)
  }

  fn leave_field(&mut self, delta: isize) {
    let current = self.field;
    if let Err(error) = self.validate_field(current) {
      self.validation_error = Some(error);
      return;
    }

    self.validation_error = None;
    let count = Self::FIELD_COUNT as isize;
    self.field = ((self.field as isize + delta).rem_euclid(count)) as usize;
    self.replace_on_type = self.is_numeric_field();
  }

  fn validate_field(&mut self, field: usize) -> Result<(), String> {
    match field {
      START_YEAR | END_YEAR => self.validate_year(field)?,
      START_MONTH | END_MONTH => self.validate_two_digit(field, 1, 12)?,
      START_DAY => {
        self.validate_two_digit(field, 1, 31)?;
        self.validate_date(true)?;
      }
      END_DAY => {
        self.validate_two_digit(field, 1, 31)?;
        self.validate_date(false)?;
      }
      START_HOUR | END_HOUR => self.validate_two_digit(field, 0, 23)?,
      START_MINUTE => {
        self.validate_two_digit(field, 0, 59)?;
        self.start_local()?;
      }
      END_MINUTE => {
        self.validate_two_digit(field, 0, 59)?;
        self.end_local()?;
      }
      AUDIENCE_FIELD | MESSAGE_FIELD => {}
      _ => return Err("unknown OOO editor field".into()),
    }
    Ok(())
  }

  fn validate_year(&mut self, field: usize) -> Result<(), String> {
    let raw = self
      .input(field)
      .map(TextInput::text)
      .unwrap_or_default();
    if raw.chars().count() != 4 || !raw.chars().all(|ch| ch.is_ascii_digit()) {
      return Err(format!("{} must contain four digits", field_label(field)));
    }
    let year = raw
      .parse::<i32>()
      .map_err(|_| format!("{} is not a valid year", field_label(field)))?;
    if !(1900..=9999).contains(&year) {
      return Err(format!("{} must be between 1900 and 9999", field_label(field)));
    }
    Ok(())
  }

  fn validate_two_digit(&mut self, field: usize, min: u32, max: u32) -> Result<(), String> {
    let raw = self
      .input(field)
      .map(TextInput::text)
      .unwrap_or_default();
    if raw.is_empty() || raw.chars().count() > 2 || !raw.chars().all(|ch| ch.is_ascii_digit()) {
      return Err(format!("{} must contain one or two digits", field_label(field)));
    }
    let value = raw
      .parse::<u32>()
      .map_err(|_| format!("{} is not a number", field_label(field)))?;
    if !(min..=max).contains(&value) {
      return Err(format!(
        "{} must be in range {min}..{max}",
        field_label(field)
      ));
    }
    let normalized = format!("{value:02}");
    if let Some(input) = self.input_mut(field) {
      *input = TextInput::from(normalized.as_str());
    }
    Ok(())
  }

  fn validate_date(&self, start: bool) -> Result<(), String> {
    let (year, month, day) = if start {
      (
        self.number(START_YEAR)? as i32,
        self.number(START_MONTH)?,
        self.number(START_DAY)?,
      )
    } else {
      (
        self.number(END_YEAR)? as i32,
        self.number(END_MONTH)?,
        self.number(END_DAY)?,
      )
    };

    NaiveDate::from_ymd_opt(year, month, day)
      .map(|_| ())
      .ok_or_else(|| {
        format!(
          "{} date {year:04}-{month:02}-{day:02} does not exist",
          if start { "Start" } else { "End" }
        )
      })
  }

  fn start_local(&self) -> Result<DateTime<Local>, String> {
    parse_local_parts(
      self.number(START_YEAR)? as i32,
      self.number(START_MONTH)?,
      self.number(START_DAY)?,
      self.number(START_HOUR)?,
      self.number(START_MINUTE)?,
      "Start",
    )
  }

  fn end_local(&self) -> Result<DateTime<Local>, String> {
    parse_local_parts(
      self.number(END_YEAR)? as i32,
      self.number(END_MONTH)?,
      self.number(END_DAY)?,
      self.number(END_HOUR)?,
      self.number(END_MINUTE)?,
      "End",
    )
  }

  fn number(&self, field: usize) -> Result<u32, String> {
    self.input(field)
      .ok_or_else(|| "OOO editor field is unavailable".to_string())?
      .text()
      .parse::<u32>()
      .map_err(|_| format!("{} is invalid", field_label(field)))
  }

  fn input(&self, field: usize) -> Option<&TextInput> {
    match field {
      START_YEAR => Some(&self.start_year),
      START_MONTH => Some(&self.start_month),
      START_DAY => Some(&self.start_day),
      START_HOUR => Some(&self.start_hour),
      START_MINUTE => Some(&self.start_minute),
      END_YEAR => Some(&self.end_year),
      END_MONTH => Some(&self.end_month),
      END_DAY => Some(&self.end_day),
      END_HOUR => Some(&self.end_hour),
      END_MINUTE => Some(&self.end_minute),
      MESSAGE_FIELD => Some(&self.message),
      _ => None,
    }
  }

  fn input_mut(&mut self, field: usize) -> Option<&mut TextInput> {
    match field {
      START_YEAR => Some(&mut self.start_year),
      START_MONTH => Some(&mut self.start_month),
      START_DAY => Some(&mut self.start_day),
      START_HOUR => Some(&mut self.start_hour),
      START_MINUTE => Some(&mut self.start_minute),
      END_YEAR => Some(&mut self.end_year),
      END_MONTH => Some(&mut self.end_month),
      END_DAY => Some(&mut self.end_day),
      END_HOUR => Some(&mut self.end_hour),
      END_MINUTE => Some(&mut self.end_minute),
      MESSAGE_FIELD => Some(&mut self.message),
      _ => None,
    }
  }
}

impl View {
  pub fn from_settings(settings: MailboxSettingsSnapshot) -> Self {
    let setting = &settings.automatic_replies_setting;
    let local_zone = local_timezone_name();
    let start_local = setting
      .scheduled_start_date_time
      .as_ref()
      .and_then(graph_datetime_to_local);
    let end_local = setting
      .scheduled_end_date_time
      .as_ref()
      .and_then(graph_datetime_to_local);

    let (start, start_detail) = display_schedule_value(
      setting.scheduled_start_date_time.as_ref(),
      start_local.as_ref(),
    );
    let (end, end_detail) =
      display_schedule_value(setting.scheduled_end_date_time.as_ref(), end_local.as_ref());

    let dst_transition = match (start_local.as_ref(), end_local.as_ref()) {
      (Some(start), Some(end)) => {
        start.offset().local_minus_utc() != end.offset().local_minus_utc()
      }
      _ => false,
    };

    let message = setting
      .internal_reply_message
      .as_deref()
      .map(html_to_plain)
      .unwrap_or_default();

    let external_differs = setting
      .external_reply_message
      .as_deref()
      .filter(|value| !value.trim().is_empty())
      .is_some_and(|value| {
        value.trim()
          != setting
            .internal_reply_message
            .as_deref()
            .unwrap_or_default()
            .trim()
      });

    Self {
      status: if setting.status.trim().is_empty() {
        "unknown".into()
      } else {
        setting.status.clone()
      },
      start,
      end,
      start_detail,
      end_detail,
      audience: audience_from_setting(setting),
      message,
      local_zone,
      mailbox_zone: settings.time_zone.clone(),
      stored_zone: setting
        .scheduled_start_date_time
        .as_ref()
        .and_then(|value| value.time_zone.clone()),
      external_differs,
      dst_transition,
      settings,
    }
  }

  pub fn text(&self) -> String {
    let mut out = String::new();
    out.push_str(&format!("Status: {}\n", self.status));
    out.push_str(&format!("Time zone: {} · DST-aware\n", self.local_zone));

    if let Some(mailbox_zone) = self.mailbox_zone.as_deref() {
      if mailbox_zone != self.local_zone {
        out.push_str(&format!("Mailbox zone: {mailbox_zone}\n"));
      }
    }

    if !self.status.eq_ignore_ascii_case("disabled") {
      if let Some(start) = self.start.as_deref() {
        out.push_str(&format!(
          "Start: {start}{}\n",
          self.start_detail
            .as_deref()
            .map(|detail| format!("  {detail}"))
            .unwrap_or_default()
        ));
      }
      if let Some(end) = self.end.as_deref() {
        out.push_str(&format!(
          "End:   {end}{}\n",
          self.end_detail
            .as_deref()
            .map(|detail| format!("  {detail}"))
            .unwrap_or_default()
        ));
      }
      if self.dst_transition {
        out.push_str("DST: transition occurs inside this interval\n");
      }
      if let Some(stored_zone) = self.stored_zone.as_deref() {
        if stored_zone != "UTC" && stored_zone != self.local_zone {
          out.push_str(&format!(
            "Stored schedule zone: {stored_zone} (displayed wall time may differ)\n"
          ));
        }
      }
      out.push_str(&format!("Audience: {}\n\n", self.audience.label()));
      out.push_str("Message:\n");
      if self.message.trim().is_empty() {
        out.push_str("(empty)");
      } else {
        out.push_str(&self.message);
      }
      if self.external_differs && self.audience == Audience::All {
        out.push_str(
          "\n\n! Existing external reply differs; editing with All will unify both messages.",
        );
      }
    }

    out.push_str("\n\ns set/edit · c clear · w Outlook Web · Esc close");
    out
  }
}

pub fn local_timezone_name() -> String {
  if let Ok(value) = std::env::var("TZ") {
    let value = value.trim().trim_start_matches(':');
    if !value.is_empty() {
      if let Some((_, zone)) = value.split_once("/zoneinfo/") {
        return zone.to_string();
      }
      return value.to_string();
    }
  }

  if let Ok(target) = std::fs::read_link("/etc/localtime") {
    let value = target.to_string_lossy();
    if let Some((_, zone)) = value.split_once("/zoneinfo/") {
      if !zone.trim().is_empty() {
        return zone.to_string();
      }
    }
  }

  Local::now().offset().to_string()
}

fn input(value: impl std::fmt::Display) -> TextInput {
  TextInput::from(value.to_string().as_str())
}

fn numeric_width(field: usize) -> usize {
  match field {
    START_YEAR | END_YEAR => 4,
    START_MONTH | START_DAY | START_HOUR | START_MINUTE | END_MONTH | END_DAY | END_HOUR
    | END_MINUTE => 2,
    _ => 0,
  }
}

fn field_label(field: usize) -> &'static str {
  match field {
    START_YEAR => "Start year",
    START_MONTH => "Start month",
    START_DAY => "Start day",
    START_HOUR => "Start hour",
    START_MINUTE => "Start minute",
    END_YEAR => "End year",
    END_MONTH => "End month",
    END_DAY => "End day",
    END_HOUR => "End hour",
    END_MINUTE => "End minute",
    AUDIENCE_FIELD => "Audience",
    MESSAGE_FIELD => "Message",
    _ => "Field",
  }
}

fn default_range() -> (DateTime<Local>, DateTime<Local>) {
  let start = Local::now() + Duration::minutes(15);
  let end = start + Duration::days(1);
  (start, end)
}

fn parse_local_parts(
  year: i32,
  month: u32,
  day: u32,
  hour: u32,
  minute: u32,
  label: &str,
) -> Result<DateTime<Local>, String> {
  let date = NaiveDate::from_ymd_opt(year, month, day)
    .ok_or_else(|| format!("{label} date {year:04}-{month:02}-{day:02} does not exist"))?;
  let time = NaiveTime::from_hms_opt(hour, minute, 0)
    .ok_or_else(|| format!("{label} time {hour:02}:{minute:02} is invalid"))?;
  let naive = date.and_time(time);

  match Local.from_local_datetime(&naive) {
    LocalResult::Single(value) => Ok(value),
    LocalResult::None => Err(format!(
      "{label} local time does not exist because of a DST change"
    )),
    LocalResult::Ambiguous(_, _) => Err(format!(
      "{label} local time is ambiguous because of a DST change"
    )),
  }
}

fn local_time_detail_value(value: &DateTime<Local>) -> String {
  let abbreviation = value.format("%Z").to_string();
  let offset = value.format("%:z").to_string();
  if abbreviation.trim().is_empty() {
    format!("UTC{offset}")
  } else {
    format!("{abbreviation} · UTC{offset}")
  }
}

fn parse_graph_naive(value: &str) -> Option<NaiveDateTime> {
  let normalized = value.trim().replace(" T ", "T");
  for format in [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%dT%H:%M",
  ] {
    if let Ok(value) = NaiveDateTime::parse_from_str(&normalized, format) {
      return Some(value);
    }
  }
  None
}

fn graph_datetime_to_local(value: &DateTimeTimeZone) -> Option<DateTime<Local>> {
  let naive = parse_graph_naive(&value.date_time)?;
  let zone = value.time_zone.as_deref().unwrap_or("").trim();

  if zone.eq_ignore_ascii_case("UTC") || zone.eq_ignore_ascii_case("Etc/UTC") {
    return Some(Utc.from_utc_datetime(&naive).with_timezone(&Local));
  }

  if zone.is_empty() || zone == local_timezone_name() {
    return match Local.from_local_datetime(&naive) {
      LocalResult::Single(value) => Some(value),
      _ => None,
    };
  }

  None
}

fn graph_datetime_for_editor(value: &DateTimeTimeZone) -> Option<DateTime<Local>> {
  if let Some(value) = graph_datetime_to_local(value) {
    return Some(value);
  }

  let naive = parse_graph_naive(&value.date_time)?;
  match Local.from_local_datetime(&naive) {
    LocalResult::Single(value) => Some(value),
    _ => None,
  }
}

fn display_schedule_value(
  raw: Option<&DateTimeTimeZone>,
  local: Option<&DateTime<Local>>,
) -> (Option<String>, Option<String>) {
  if let Some(local) = local {
    return (
      Some(local.format("%Y-%m-%d %H:%M").to_string()),
      Some(local_time_detail_value(local)),
    );
  }

  let Some(raw) = raw else {
    return (None, None);
  };
  let Some(naive) = parse_graph_naive(&raw.date_time) else {
    return (
      Some(raw.date_time.clone()),
      raw.time_zone
        .as_deref()
        .map(|zone| format!("stored zone: {zone}")),
    );
  };
  (
    Some(naive.format("%Y-%m-%d %H:%M").to_string()),
    raw.time_zone
      .as_deref()
      .map(|zone| format!("stored zone: {zone}")),
  )
}

fn audience_from_setting(setting: &AutomaticRepliesSetting) -> Audience {
  if setting
    .external_audience
    .as_deref()
    .is_some_and(|value| value.eq_ignore_ascii_case("all"))
  {
    Audience::All
  } else {
    Audience::Internal
  }
}

fn html_to_plain(value: &str) -> String {
  let rendered = content::render_body(Some("html"), value);
  content::plain(&rendered.text).trim().to_string()
}

#[cfg(test)]
mod template_tests {
  use super::*;

  fn local_datetime(year: i32, month: u32, day: u32) -> DateTime<Local> {
    Local
      .with_ymd_and_hms(year, month, day, 9, 0, 0)
      .single()
      .expect("test date must exist in local time")
  }

  fn template_state(template: &str, last_rendered: &str) -> TemplateState {
    TemplateState {
      template: template.into(),
      locale: Locale::POSIX.to_string(),
      markdown: false,
      last_rendered: last_rendered.into(),
    }
  }

  #[test]
  fn expands_start_end_with_non_padded_dates() {
    let start = local_datetime(2026, 10, 28);
    let end = local_datetime(2026, 11, 1);
    let rendered = render_template_message(
      "Away {st %-d.%-m} - {end %-d.%-m}",
      &start,
      &end,
      Locale::POSIX,
      false,
    )
    .unwrap();
    assert_eq!(rendered.plain, "Away 28.10 - 1.11");
  }

  #[test]
  fn localized_textual_day_and_month_are_supported() {
    let start = local_datetime(2026, 11, 13);
    let end = local_datetime(2026, 11, 17);
    let rendered = render_template_message(
      "{st %A %-d. %B}",
      &start,
      &end,
      Locale::cs_CZ,
      false,
    )
    .unwrap();
    assert!(rendered.plain.contains("13."));
    assert!(!rendered.plain.contains("Friday"));
    assert!(!rendered.plain.contains("November"));
  }

  #[test]
  fn malformed_known_placeholder_is_rejected() {
    let start = local_datetime(2026, 11, 13);
    let end = local_datetime(2026, 11, 17);
    assert!(render_template_message(
      "Away until {end %Q}",
      &start,
      &end,
      Locale::POSIX,
      false,
    )
    .is_err());
    assert!(render_template_message(
      "Away until {end %d.%m",
      &start,
      &end,
      Locale::POSIX,
      false,
    )
    .is_err());
  }

  #[test]
  fn unrelated_braces_remain_literal() {
    let start = local_datetime(2026, 11, 13);
    let end = local_datetime(2026, 11, 17);
    let rendered = render_template_message(
      "Ticket {customer} until {end %-d.%-m}",
      &start,
      &end,
      Locale::POSIX,
      false,
    )
    .unwrap();
    assert_eq!(rendered.plain, "Ticket {customer} until 17.11");
  }

  #[test]
  fn restores_previous_render_after_schedule_only_change() {
    let old_start = local_datetime(2026, 10, 28);
    let old_end = local_datetime(2026, 11, 1);
    let new_start = local_datetime(2026, 11, 13);
    let new_end = local_datetime(2026, 11, 17);
    let template = "Away {st %-d.%-m} - {end %-d.%-m} incl. Contact colleagues.";
    let old = render_template_message(template, &old_start, &old_end, Locale::POSIX, false)
      .unwrap()
      .plain;
    let state = template_state(template, &old);
    let restored = restore_template(&state, &old, &new_start, &new_end).unwrap();
    assert_eq!(restored.1, TemplateRestore::Previous);
  }

  #[test]
  fn fuzzy_restore_accepts_changed_slot_values_but_not_changed_body() {
    let start = local_datetime(2026, 11, 13);
    let end = local_datetime(2026, 11, 17);
    let template = "Hello, away {st %-d.%-m} - {end %-d.%-m} incl. Contact colleagues. Petr";
    let state = template_state(template, "something deliberately different");

    let restored = restore_template(
      &state,
      "Hello, away 28.10 - 1.11 incl. Contact colleagues. Petr",
      &start,
      &end,
    )
    .unwrap();
    assert_eq!(restored.1, TemplateRestore::Structure);

    assert!(restore_template(
      &state,
      "Hello, away 28.10 - 1.11 incl. Contact the manager. Petr",
      &start,
      &end,
    )
    .is_none());
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn settings() -> MailboxSettingsSnapshot {
    MailboxSettingsSnapshot {
      time_zone: Some("test-zone".into()),
      automatic_replies_setting: AutomaticRepliesSetting::default(),
    }
  }

  #[test]
  fn invalid_month_is_rejected_on_field_leave() {
    let mut editor = Editor::from_settings(&settings());
    editor.field = START_MONTH;
    editor.start_month = TextInput::from("13");
    editor.next_field();
    assert_eq!(editor.field, START_MONTH);
    assert!(editor.validation_error.is_some());
  }

  #[test]
  fn valid_single_digit_month_is_normalized_on_leave() {
    let mut editor = Editor::from_settings(&settings());
    editor.field = START_MONTH;
    editor.start_month = TextInput::from("7");
    editor.next_field();
    assert_eq!(editor.field, START_DAY);
    assert_eq!(editor.start_month.text(), "07");
    assert!(editor.validation_error.is_none());
  }

  #[test]
  fn impossible_date_is_rejected_when_leaving_day() {
    let mut editor = Editor::from_settings(&settings());
    editor.start_year = TextInput::from("2026");
    editor.start_month = TextInput::from("02");
    editor.start_day = TextInput::from("30");
    editor.field = START_DAY;
    editor.next_field();
    assert_eq!(editor.field, START_DAY);
    assert!(editor
      .validation_error
      .as_deref()
      .is_some_and(|error| error.contains("does not exist")));
  }

  #[test]
  fn audience_maps_only_internal_or_all() {
    let mut setting = AutomaticRepliesSetting::default();
    assert_eq!(audience_from_setting(&setting), Audience::Internal);
    setting.external_audience = Some("all".into());
    assert_eq!(audience_from_setting(&setting), Audience::All);
  }

  #[test]
  fn graph_datetime_parser_accepts_fractional_seconds() {
    let value = parse_graph_naive("2026-10-21T12:34:56.0000000").unwrap();
    assert_eq!(
      value.format("%Y-%m-%d %H:%M").to_string(),
      "2026-10-21 12:34"
    );
  }
}
