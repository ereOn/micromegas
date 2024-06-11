use crate::scope::ScopeDesc;
use crate::{fetch_block_payload, parse_block};
use anyhow::{Context, Result};
use micromegas_telemetry::blob_storage::BlobStorage;
use micromegas_telemetry::stream_info::StreamInfo;
use micromegas_tracing::prelude::*;
use micromegas_tracing::warn;
use micromegas_transit::{Object, Value};
use std::sync::Arc;

pub trait ThreadBlockProcessor {
    // return true to continue
    fn on_begin_thread_scope(&mut self, event_id: i64, scope: ScopeDesc, ts: i64) -> Result<bool>;
    fn on_end_thread_scope(&mut self, event_id: i64, scope: ScopeDesc, ts: i64) -> Result<bool>;
}

fn on_thread_event<F>(obj: &micromegas_transit::Object, mut fun: F) -> Result<bool>
where
    F: FnMut(Arc<Object>, i64) -> Result<bool>,
{
    let tick = obj.get::<i64>("time")?;
    let scope = obj.get::<Arc<Object>>("thread_span_desc")?;
    fun(scope, tick)
}

fn on_thread_named_event<F>(obj: &micromegas_transit::Object, mut fun: F) -> Result<bool>
where
    F: FnMut(Arc<Object>, Arc<String>, i64) -> Result<bool>,
{
    let tick = obj.get::<i64>("time")?;
    let scope = obj.get::<Arc<Object>>("thread_span_location")?;
    let name = obj.get::<Arc<String>>("name")?;
    fun(scope, name, tick)
}

#[span_fn]
pub fn parse_thread_block_payload<Proc: ThreadBlockProcessor>(
    object_offset: i64,
    payload: &micromegas_telemetry::block_wire_format::BlockPayload,
    stream: &micromegas_telemetry::stream_info::StreamInfo,
    processor: &mut Proc,
) -> Result<bool> {
    let mut event_id = object_offset;
    parse_block(stream, payload, |val| {
        let res = if let Value::Object(obj) = val {
            match obj.type_name.as_str() {
                "BeginThreadSpanEvent" => on_thread_event(&obj, |scope, ts| {
                    let name = scope.get::<Arc<String>>("name")?;
                    let filename = scope.get::<Arc<String>>("file")?;
                    let target = scope.get::<Arc<String>>("target")?;
                    let line = scope.get::<u32>("line")?;
                    let scope_desc = ScopeDesc::new(name, filename, target, line);
                    processor.on_begin_thread_scope(event_id, scope_desc, ts)
                })
                .with_context(|| "reading BeginThreadSpanEvent"),
                "EndThreadSpanEvent" => on_thread_event(&obj, |scope, ts| {
                    let name = scope.get::<Arc<String>>("name")?;
                    let filename = scope.get::<Arc<String>>("file")?;
                    let target = scope.get::<Arc<String>>("target")?;
                    let line = scope.get::<u32>("line")?;
                    let scope_desc = ScopeDesc::new(name, filename, target, line);
                    processor.on_end_thread_scope(event_id, scope_desc, ts)
                })
                .with_context(|| "reading EndThreadSpanEvent"),
                "BeginThreadNamedSpanEvent" => on_thread_named_event(&obj, |scope, name, ts| {
                    let filename = scope.get::<Arc<String>>("file")?;
                    let target = scope.get::<Arc<String>>("target")?;
                    let line = scope.get::<u32>("line")?;
                    let scope_desc = ScopeDesc::new(name, filename, target, line);
                    processor.on_begin_thread_scope(event_id, scope_desc, ts)
                })
                .with_context(|| "reading BeginThreadNamedSpanEvent"),
                "EndThreadNamedSpanEvent" => on_thread_named_event(&obj, |scope, name, ts| {
                    let filename = scope.get::<Arc<String>>("file")?;
                    let target = scope.get::<Arc<String>>("target")?;
                    let line = scope.get::<u32>("line")?;
                    let scope_desc = ScopeDesc::new(name, filename, target, line);
                    processor.on_end_thread_scope(event_id, scope_desc, ts)
                })
                .with_context(|| "reading EndThreadNamedSpanEvent"),
                event_type => {
                    warn!("unknown event type {}", event_type);
                    Ok(true)
                }
            }
        } else {
            Ok(true) // continue
        };
        event_id += 1;
        res
    })
}

#[span_fn]
pub async fn parse_thread_block<Proc: ThreadBlockProcessor>(
    blob_storage: Arc<BlobStorage>,
    stream: &StreamInfo,
    block_id: sqlx::types::Uuid,
    object_offset: i64,
    processor: &mut Proc,
) -> Result<bool> {
    let payload =
        fetch_block_payload(blob_storage, stream.process_id, stream.stream_id, block_id).await?;
    parse_thread_block_payload(object_offset, &payload, stream, processor)
}
