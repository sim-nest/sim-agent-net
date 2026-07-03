//! Deterministic synthetic vision, speech, and sensor fixtures for local recipes.

use sim_codec_chat::{model_error_expr, model_response_expr};
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelCard, ModelRequest, ModelResponse, ModelRunner};
use std::collections::BTreeMap;

/// Scripted output for one synthetic image reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeVisionFixture {
    /// Scripted caption returned for the image.
    pub caption: String,
    /// Additional deterministic name/value fields.
    pub fields: Vec<(String, String)>,
}

impl FakeVisionFixture {
    /// Builds a vision fixture from a caption.
    pub fn new(caption: impl Into<String>) -> Self {
        Self {
            caption: caption.into(),
            fields: Vec::new(),
        }
    }

    /// Adds one deterministic field to the fixture.
    pub fn with_field(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.push((name.into(), value.into()));
        self
    }
}

/// Local runner that maps synthetic image references to scripted vision output.
#[derive(Clone, Debug)]
pub struct FakeVisionRunner {
    model: String,
    fixtures: BTreeMap<String, FakeVisionFixture>,
}

impl FakeVisionRunner {
    /// Creates an empty fake vision runner for the named model surface.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            fixtures: BTreeMap::new(),
        }
    }

    /// Adds one synthetic image fixture.
    pub fn with_fixture(
        mut self,
        reference: impl Into<String>,
        fixture: FakeVisionFixture,
    ) -> Result<Self> {
        let reference = reference.into();
        validate_synthetic_reference(&reference)?;
        self.fixtures.insert(reference, fixture);
        Ok(self)
    }

    /// Returns a fixture by reference.
    pub fn fixture(&self, reference: &str) -> Option<&FakeVisionFixture> {
        self.fixtures.get(reference)
    }
}

impl ModelRunner for FakeVisionRunner {
    fn card(&self) -> ModelCard {
        model_card(
            Symbol::qualified("runner", "fake-vision"),
            self.model.clone(),
            Symbol::new("fake-vision"),
            ["image"],
            ["text", "fields"],
        )
    }

    fn infer(&self, _cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let Some(reference) = request_reference(&request, &["image", "image-ref", "fixture"])
        else {
            return error_response(
                Symbol::qualified("runner", "fake-vision"),
                &self.model,
                "fake vision request missing synthetic image reference",
            );
        };
        if let Err(error) = validate_synthetic_reference(reference) {
            return error_response(
                Symbol::qualified("runner", "fake-vision"),
                &self.model,
                error.to_string(),
            );
        }
        let Some(fixture) = self.fixtures.get(reference) else {
            return error_response(
                Symbol::qualified("runner", "fake-vision"),
                &self.model,
                format!("fake vision fixture not found: {reference}"),
            );
        };
        response_from_parts(
            Symbol::qualified("runner", "fake-vision"),
            &self.model,
            vec![
                text_part(&fixture.caption),
                string_part("fixture", "reference", reference),
                fields_part(&fixture.fields),
            ],
        )
    }
}

/// One deterministic speech transcript segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeAsrSegment {
    /// Segment start offset in milliseconds.
    pub start_ms: u64,
    /// Segment end offset in milliseconds, at or after the start.
    pub end_ms: u64,
    /// Transcribed text for this segment.
    pub text: String,
}

impl FakeAsrSegment {
    /// Builds one transcript segment with fixed offsets.
    pub fn new(start_ms: u64, end_ms: u64, text: impl Into<String>) -> Result<Self> {
        if end_ms < start_ms {
            return Err(Error::Eval(
                "fake ASR segment end must be at or after start".to_owned(),
            ));
        }
        Ok(Self {
            start_ms,
            end_ms,
            text: text.into(),
        })
    }
}

/// Scripted output for one synthetic audio reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeAsrFixture {
    /// Full scripted transcript for the audio.
    pub transcript: String,
    /// Locale tag for the transcript.
    pub locale: String,
    /// Per-segment breakdown of the transcript.
    pub segments: Vec<FakeAsrSegment>,
}

impl FakeAsrFixture {
    /// Builds an ASR fixture from a transcript and locale.
    pub fn new(transcript: impl Into<String>, locale: impl Into<String>) -> Self {
        Self {
            transcript: transcript.into(),
            locale: locale.into(),
            segments: Vec::new(),
        }
    }

    /// Adds one deterministic transcript segment.
    pub fn with_segment(mut self, segment: FakeAsrSegment) -> Self {
        self.segments.push(segment);
        self
    }
}

/// Local runner that maps synthetic audio references to scripted transcripts.
#[derive(Clone, Debug)]
pub struct FakeAsrRunner {
    model: String,
    fixtures: BTreeMap<String, FakeAsrFixture>,
}

impl FakeAsrRunner {
    /// Creates an empty fake ASR runner for the named model surface.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            fixtures: BTreeMap::new(),
        }
    }

    /// Adds one synthetic audio fixture.
    pub fn with_fixture(
        mut self,
        reference: impl Into<String>,
        fixture: FakeAsrFixture,
    ) -> Result<Self> {
        let reference = reference.into();
        validate_synthetic_reference(&reference)?;
        self.fixtures.insert(reference, fixture);
        Ok(self)
    }

    /// Returns a fixture by reference.
    pub fn fixture(&self, reference: &str) -> Option<&FakeAsrFixture> {
        self.fixtures.get(reference)
    }
}

impl ModelRunner for FakeAsrRunner {
    fn card(&self) -> ModelCard {
        model_card(
            Symbol::qualified("runner", "fake-asr"),
            self.model.clone(),
            Symbol::new("fake-asr"),
            ["audio"],
            ["text", "transcript"],
        )
    }

    fn infer(&self, _cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        let Some(reference) = request_reference(&request, &["audio", "audio-ref", "fixture"])
        else {
            return error_response(
                Symbol::qualified("runner", "fake-asr"),
                &self.model,
                "fake ASR request missing synthetic audio reference",
            );
        };
        if let Err(error) = validate_synthetic_reference(reference) {
            return error_response(
                Symbol::qualified("runner", "fake-asr"),
                &self.model,
                error.to_string(),
            );
        }
        let Some(fixture) = self.fixtures.get(reference) else {
            return error_response(
                Symbol::qualified("runner", "fake-asr"),
                &self.model,
                format!("fake ASR fixture not found: {reference}"),
            );
        };
        response_from_parts(
            Symbol::qualified("runner", "fake-asr"),
            &self.model,
            vec![
                text_part(&fixture.transcript),
                string_part("fixture", "reference", reference),
                transcript_part(fixture),
            ],
        )
    }
}

/// One fixed sensor frame in a synthetic sensor stream.
#[derive(Clone, Debug, PartialEq)]
pub struct FakeSensorFrame {
    /// Monotonic sequence number of this frame in the stream.
    pub sequence: u64,
    /// Named sensor readings, each a finite value.
    pub readings: Vec<(String, f64)>,
}

impl FakeSensorFrame {
    /// Builds one sensor frame and rejects non-finite readings.
    pub fn new(sequence: u64, readings: Vec<(String, f64)>) -> Result<Self> {
        if readings.iter().any(|(_, value)| !value.is_finite()) {
            return Err(Error::Eval(
                "fake sensor frame readings must be finite".to_owned(),
            ));
        }
        Ok(Self { sequence, readings })
    }

    /// Encodes the frame as an expression for recipe fixtures and traces.
    pub fn as_expr(&self) -> Expr {
        Expr::Map(vec![
            number_entry("sequence", self.sequence),
            (
                sym("readings"),
                Expr::Map(
                    self.readings
                        .iter()
                        .map(|(name, value)| {
                            (
                                Expr::String(name.clone()),
                                number_literal("f64", value.to_string()),
                            )
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// Fixed local stream over synthetic sensor frames.
#[derive(Clone, Debug, PartialEq)]
pub struct FakeSensorStream {
    frames: Vec<FakeSensorFrame>,
    cursor: usize,
}

impl FakeSensorStream {
    /// Builds a fixed sensor stream from pre-authored frames.
    pub fn new(frames: Vec<FakeSensorFrame>) -> Result<Self> {
        if frames.is_empty() {
            return Err(Error::Eval(
                "fake sensor stream needs at least one frame".to_owned(),
            ));
        }
        Ok(Self { frames, cursor: 0 })
    }

    /// Returns the next frame and advances the stream.
    pub fn next_frame(&mut self) -> Option<FakeSensorFrame> {
        let frame = self.frames.get(self.cursor).cloned()?;
        self.cursor += 1;
        Some(frame)
    }

    /// Resets the cursor to the start of the fixed sequence.
    pub fn reset(&mut self) {
        self.cursor = 0;
    }

    /// Returns all fixed frames without advancing the stream.
    pub fn frames(&self) -> &[FakeSensorFrame] {
        &self.frames
    }
}

impl Iterator for FakeSensorStream {
    type Item = FakeSensorFrame;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_frame()
    }
}

fn validate_synthetic_reference(reference: &str) -> Result<()> {
    if reference.starts_with("fixture:") || reference.starts_with("synthetic:") {
        return Ok(());
    }
    Err(Error::Eval(
        "fake multimodal fixtures require fixture: or synthetic: references".to_owned(),
    ))
}

fn request_reference<'a>(request: &'a ModelRequest, keys: &[&str]) -> Option<&'a str> {
    expr_reference(&request.task, keys).or_else(|| {
        request
            .messages
            .iter()
            .find_map(|expr| expr_reference(expr, keys))
    })
}

fn expr_reference<'a>(expr: &'a Expr, keys: &[&str]) -> Option<&'a str> {
    match expr {
        Expr::String(text) if is_fixture_reference(text) => Some(text.as_str()),
        Expr::Map(entries) => entries.iter().find_map(|(key, value)| {
            if key_matches(key, keys)
                && let Expr::String(reference) = value
            {
                return Some(reference.as_str());
            }
            expr_reference(value, keys)
        }),
        Expr::List(items) | Expr::Vector(items) => {
            items.iter().find_map(|item| expr_reference(item, keys))
        }
        _ => None,
    }
}

fn is_fixture_reference(text: &str) -> bool {
    text.starts_with("fixture:") || text.starts_with("synthetic:")
}

fn key_matches(expr: &Expr, keys: &[&str]) -> bool {
    match expr {
        Expr::Symbol(symbol) => {
            let name = symbol.as_qualified_str();
            keys.iter().any(|key| *key == name)
        }
        Expr::String(text) => keys.iter().any(|key| *key == text),
        _ => false,
    }
}

fn response_from_parts(runner: Symbol, model: &str, parts: Vec<Expr>) -> Result<ModelResponse> {
    ModelResponse::try_from(model_response_expr(
        runner,
        model,
        parts,
        Symbol::new("stop"),
    ))
}

fn error_response(
    runner: Symbol,
    model: &str,
    message: impl Into<String>,
) -> Result<ModelResponse> {
    ModelResponse::try_from(model_error_expr(runner, model, message))
}

fn model_card<const IN: usize, const OUT: usize>(
    runner: Symbol,
    model: String,
    provider: Symbol,
    modalities_in: [&str; IN],
    modalities_out: [&str; OUT],
) -> ModelCard {
    let mut card = ModelCard::new(runner, model, provider, Symbol::new("local"));
    card.extra = vec![
        symbol_list_entry("modalities-in", modalities_in),
        symbol_list_entry("modalities-out", modalities_out),
        bool_entry("supports-stream", false),
        bool_entry("supports-tools", false),
        bool_entry("supports-json", true),
        bool_entry("supports-shape", false),
        symbol_entry("health", "ok"),
    ];
    card
}

use sim_codec_chat::text_part;

fn string_part(kind: &str, key: &str, value: &str) -> Expr {
    Expr::Map(vec![symbol_entry("type", kind), string_entry(key, value)])
}

fn fields_part(fields: &[(String, String)]) -> Expr {
    Expr::Map(vec![
        symbol_entry("type", "fields"),
        (
            sym("fields"),
            Expr::Map(
                fields
                    .iter()
                    .map(|(key, value)| (Expr::String(key.clone()), Expr::String(value.clone())))
                    .collect(),
            ),
        ),
    ])
}

fn transcript_part(fixture: &FakeAsrFixture) -> Expr {
    Expr::Map(vec![
        symbol_entry("type", "transcript"),
        string_entry("locale", &fixture.locale),
        (
            sym("segments"),
            Expr::List(fixture.segments.iter().map(segment_expr).collect()),
        ),
    ])
}

fn segment_expr(segment: &FakeAsrSegment) -> Expr {
    Expr::Map(vec![
        number_entry("start-ms", segment.start_ms),
        number_entry("end-ms", segment.end_ms),
        string_entry("text", &segment.text),
    ])
}

fn symbol_list_entry<const N: usize>(key: &str, values: [&str; N]) -> (Expr, Expr) {
    (
        Expr::Symbol(Symbol::new(key)),
        Expr::List(
            values
                .into_iter()
                .map(|value| Expr::Symbol(Symbol::new(value)))
                .collect(),
        ),
    )
}

fn bool_entry(key: &str, value: bool) -> (Expr, Expr) {
    (sym(key), Expr::Bool(value))
}

fn number_entry(key: &str, value: u64) -> (Expr, Expr) {
    (sym(key), number_literal("i64", value.to_string()))
}

fn symbol_entry(key: &str, value: &str) -> (Expr, Expr) {
    (sym(key), Expr::Symbol(Symbol::new(value)))
}

fn string_entry(key: &str, value: &str) -> (Expr, Expr) {
    (sym(key), Expr::String(value.to_owned()))
}

use sim_value::build::sym;

fn number_literal(domain: &str, canonical: String) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", domain),
        canonical,
    })
}
