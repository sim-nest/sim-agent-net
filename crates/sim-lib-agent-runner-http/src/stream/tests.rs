use super::*;

#[derive(Default)]
struct CollectEvents {
    events: Vec<ModelEvent>,
}

impl ModelEventSink for CollectEvents {
    fn emit(&mut self, event: ModelEvent) -> Result<()> {
        self.events.push(event);
        Ok(())
    }
}

#[test]
fn openai_finish_emits_unterminated_delta_to_sink() {
    let mut decoder =
        HttpStreamDecoder::openai(Symbol::new("http-openai"), "gpt-test".to_owned(), true);
    let mut sink = CollectEvents::default();
    decoder
        .feed(
            br#"data: {"choices":[{"index":0,"delta":{"content":"tail"},"finish_reason":null}]}"#,
            &mut sink,
        )
        .unwrap();
    assert!(sink.events.is_empty());
    assert!(decoder.has_stream_output());

    let response = decoder.finish(&mut sink).unwrap();

    assert_eq!(sink.events.len(), 1);
    assert_eq!(sink.events[0].event, Symbol::new("delta"));
    assert_eq!(
        sink.events[0].extra,
        vec![(
            Expr::Symbol(Symbol::new("text")),
            Expr::String("tail".to_owned()),
        )]
    );
    assert!(format!("{:?}", response.content).contains("tail"));
    assert_eq!(response.extra.len(), 1);
}

#[test]
fn ollama_finish_emits_unterminated_delta_to_sink() {
    let mut decoder =
        HttpStreamDecoder::ollama(Symbol::new("http-ollama"), "qwen-test".to_owned(), false);
    let mut sink = CollectEvents::default();
    decoder
        .feed(
            br#"{"model":"qwen-test","message":{"role":"assistant","content":"tail"},"done":false}"#,
            &mut sink,
        )
        .unwrap();
    assert!(sink.events.is_empty());
    assert!(decoder.has_stream_output());

    let response = decoder.finish(&mut sink).unwrap();

    assert_eq!(sink.events.len(), 1);
    assert_eq!(sink.events[0].event, Symbol::new("delta"));
    assert!(format!("{:?}", sink.events[0].extra).contains("tail"));
    assert!(format!("{:?}", response.content).contains("tail"));
}

#[test]
fn openai_stream_rejects_oversize_unterminated_line() {
    let mut decoder =
        HttpStreamDecoder::openai(Symbol::new("http-openai"), "gpt-test".to_owned(), false);
    let mut sink = CollectEvents::default();
    let too_long = vec![b'x'; MAX_STREAM_LINE_BYTES + 1];

    let error = decoder.feed(&too_long, &mut sink).unwrap_err();

    assert!(format!("{error}").contains("openai stream line exceeded size limit"));
    assert!(sink.events.is_empty());
}

#[test]
fn ollama_stream_rejects_oversize_unterminated_line() {
    let mut decoder =
        HttpStreamDecoder::ollama(Symbol::new("http-ollama"), "qwen-test".to_owned(), false);
    let mut sink = CollectEvents::default();
    let too_long = vec![b'x'; MAX_STREAM_LINE_BYTES + 1];

    let error = decoder.feed(&too_long, &mut sink).unwrap_err();

    assert!(format!("{error}").contains("ollama stream line exceeded size limit"));
    assert!(sink.events.is_empty());
}
