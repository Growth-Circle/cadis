use criterion::{black_box, criterion_group, criterion_main, Criterion};

use cadis_protocol::{
    CadisEvent, ContentKind, EventEnvelope, EventId, MessageDeltaPayload, RiskClass, Timestamp,
    ToolCallId, ToolEventPayload,
};

fn make_delta_event() -> EventEnvelope {
    EventEnvelope::new(
        EventId::new("evt-bench-001"),
        Timestamp::new_utc("2026-04-28T00:00:00Z").unwrap(),
        "cadisd",
        None,
        CadisEvent::MessageDelta(MessageDeltaPayload {
            delta: "hello".into(),
            content_kind: ContentKind::Chat,
            agent_id: None,
            agent_name: None,
            model: None,
        }),
    )
}

fn make_tool_event() -> EventEnvelope {
    EventEnvelope::new(
        EventId::new("evt-bench-002"),
        Timestamp::new_utc("2026-04-28T00:00:00Z").unwrap(),
        "cadisd",
        None,
        CadisEvent::ToolCompleted(ToolEventPayload {
            tool_call_id: ToolCallId::new("tc-001"),
            tool_name: "file.read".into(),
            summary: Some("read ok".into()),
            risk_class: Some(RiskClass::SafeRead),
            output: None,
        }),
    )
}

fn bench_event_serialize(c: &mut Criterion) {
    let event = make_delta_event();
    c.bench_function("event_serialize_delta", |b| {
        b.iter(|| serde_json::to_string(black_box(&event)).unwrap())
    });
}

fn bench_event_roundtrip(c: &mut Criterion) {
    let event = make_delta_event();
    let json = serde_json::to_string(&event).unwrap();
    c.bench_function("event_roundtrip_delta", |b| {
        b.iter(|| {
            let e: EventEnvelope = serde_json::from_str(black_box(&json)).unwrap();
            let _ = serde_json::to_string(&e).unwrap();
        })
    });
}

fn bench_tool_dispatch_serialize(c: &mut Criterion) {
    let event = make_tool_event();
    c.bench_function("tool_dispatch_serialize", |b| {
        b.iter(|| serde_json::to_string(black_box(&event)).unwrap())
    });
}

fn bench_tool_dispatch_roundtrip(c: &mut Criterion) {
    let event = make_tool_event();
    let json = serde_json::to_string(&event).unwrap();
    c.bench_function("tool_dispatch_roundtrip", |b| {
        b.iter(|| {
            let e: EventEnvelope = serde_json::from_str(black_box(&json)).unwrap();
            let _ = serde_json::to_string(&e).unwrap();
        })
    });
}

criterion_group!(
    benches,
    bench_event_serialize,
    bench_event_roundtrip,
    bench_tool_dispatch_serialize,
    bench_tool_dispatch_roundtrip,
);
criterion_main!(benches);
