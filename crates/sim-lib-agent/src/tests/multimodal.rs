use super::support::{eval_cx, flatten_text};
use crate::{
    FakeAsrFixture, FakeAsrRunner, FakeAsrSegment, FakeSensorFrame, FakeSensorStream,
    FakeVisionFixture, FakeVisionRunner, ModelRequest, ModelRunner,
};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Expr, Symbol};

#[test]
fn fake_vision_runner_maps_synthetic_image_to_caption_and_fields() {
    let runner = FakeVisionRunner::new("fake-vision/model")
        .with_fixture(
            "fixture:image:dashboard",
            FakeVisionFixture::new("dashboard shows green deployment status")
                .with_field("status", "green")
                .with_field("owner", "ops"),
        )
        .unwrap();
    let mut cx = eval_cx();

    let response = runner
        .infer(
            &mut cx,
            ModelRequest::new(
                Expr::Map(vec![(
                    Expr::Symbol(Symbol::new("image-ref")),
                    Expr::String("fixture:image:dashboard".to_owned()),
                )]),
                Vec::new(),
            ),
        )
        .unwrap();

    let expr: Expr = response.clone().into();
    validate_chat_transcript(&expr).unwrap();
    let text = flatten_text(&expr);
    assert!(text.contains("green deployment status"));
    assert!(text.contains("owner"));
    assert_eq!(
        runner
            .card()
            .extra
            .iter()
            .find(|(key, _)| *key == Expr::Symbol(Symbol::new("modalities-in")))
            .map(|(_, value)| value),
        Some(&Expr::List(vec![Expr::Symbol(Symbol::new("image"))]))
    );
}

#[test]
fn fake_asr_runner_maps_synthetic_audio_to_transcript_segments() {
    let runner = FakeAsrRunner::new("fake-asr/model")
        .with_fixture(
            "fixture:audio:briefing",
            FakeAsrFixture::new("system nominal", "en-US")
                .with_segment(FakeAsrSegment::new(0, 500, "system").unwrap())
                .with_segment(FakeAsrSegment::new(500, 900, "nominal").unwrap()),
        )
        .unwrap();
    let mut cx = eval_cx();

    let response = runner
        .infer(
            &mut cx,
            ModelRequest::new(
                Expr::Map(vec![(
                    Expr::Symbol(Symbol::new("audio-ref")),
                    Expr::String("fixture:audio:briefing".to_owned()),
                )]),
                Vec::new(),
            ),
        )
        .unwrap();

    assert!(response.content.iter().any(|part| {
        matches!(part, Expr::Map(entries) if entries.iter().any(|(key, value)| {
            *key == Expr::Symbol(Symbol::new("locale")) && *value == Expr::String("en-US".to_owned())
        }))
    }));
    let expr: Expr = response.into();
    validate_chat_transcript(&expr).unwrap();
    let text = flatten_text(&expr);
    assert!(text.contains("system nominal"));
    assert!(text.contains("500"));
}

#[test]
fn fake_sensor_stream_replays_a_fixed_frame_sequence() {
    let frames = vec![
        FakeSensorFrame::new(0, vec![("distance-cm".to_owned(), 42.0)]).unwrap(),
        FakeSensorFrame::new(1, vec![("distance-cm".to_owned(), 40.5)]).unwrap(),
    ];
    let mut stream = FakeSensorStream::new(frames.clone()).unwrap();

    assert_eq!(stream.frames(), frames.as_slice());
    assert_eq!(stream.next_frame(), Some(frames[0].clone()));
    assert_eq!(stream.next_frame(), Some(frames[1].clone()));
    assert_eq!(stream.next_frame(), None);
    stream.reset();
    assert_eq!(stream.next(), Some(frames[0].clone()));
    assert!(flatten_text(&frames[0].as_expr()).contains("distance-cm"));
}

#[test]
fn fake_multimodal_fixtures_reject_external_or_live_inputs() {
    assert!(
        FakeVisionRunner::new("fake-vision/model")
            .with_fixture(
                "https://example.invalid/image.png",
                FakeVisionFixture::new("external")
            )
            .is_err()
    );
    assert!(
        FakeAsrRunner::new("fake-asr/model")
            .with_fixture(
                "file:///tmp/audio.wav",
                FakeAsrFixture::new("external", "en-US")
            )
            .is_err()
    );
    assert!(FakeAsrSegment::new(20, 10, "bad").is_err());
    assert!(FakeSensorFrame::new(0, vec![("temperature".to_owned(), f64::NAN)]).is_err());

    let runner = FakeVisionRunner::new("fake-vision/model");
    let mut cx = eval_cx();
    let response = runner
        .infer(
            &mut cx,
            ModelRequest::new(
                Expr::String("https://example.invalid/image.png".to_owned()),
                vec![],
            ),
        )
        .unwrap();
    let expr: Expr = response.into();
    validate_chat_transcript(&expr).unwrap();
    assert!(flatten_text(&expr).contains("missing synthetic image reference"));
}
