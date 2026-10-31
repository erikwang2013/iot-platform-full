use crate::models::EventMessage;
use crate::td::event_to_insert;
use ecat_data::TsdbClient;
use ecat_data_tdengine::TdengineClient;
use ecat_mq::MessageQueue;
use ecat_mq_kafka::KafkaMq;
use futures_util::{StreamExt, stream::poll_fn};
use std::sync::Arc;

pub use ecat_iot::TOPIC_EVENTS;

/// 每批最多攒多少条事件后写一次（一次 REST 请求多行 INSERT）。
const BATCH_SIZE: usize = 100;
/// 缓冲上限（条）：TDengine 长时间不可用时丢弃最旧事件，保证内存有界
/// （有界丢失优于 OOM；丢弃量与 write failure 指标可在告警中观察）。
const MAX_BUFFER: usize = 10_000;
/// 写失败后事件路径的重试间隔（ticker 仍每秒补试）。
const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

pub fn batch_sql(events: &[EventMessage]) -> String {
    events
        .iter()
        .map(event_to_insert)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 后台任务：订阅 iot.events，攒批写入 TDengine。
/// 消费语义 at-least-once（auto_commit=true，librdkafka 每 5s 提交 offset，
/// 重启从最近提交点继续，停机窗口不再跳过）；同 ts 同 tags 覆盖保证幂等。
/// 多副本消费需显式 KafkaConfig.group_id（见 ecat-mq-kafka）。
pub async fn run(td: Arc<TdengineClient>, kafka: Arc<KafkaMq>) {
    let mut stream = match kafka.subscribe(TOPIC_EVENTS).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "kafka subscribe failed, ingest exits");
            return;
        }
    };
    let mut stream = poll_fn(move |cx| stream.poll_recv(cx)).boxed();
    let mut buf: Vec<EventMessage> = Vec::with_capacity(BATCH_SIZE);
    // 低事件量（< BATCH_SIZE）时靠时间冲刷，防止数据无限滞留内存
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    let mut next_retry = tokio::time::Instant::now();
    loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(raw)) => match serde_json::from_slice::<EventMessage>(&raw) {
                    Ok(ev) => {
                        buf.push(ev);
                        trim_buffer(&mut buf);
                        // 失败后 1s 内不再由事件触发重试，避免 TDengine 宕机时
                        // 每条事件都重建一次全量 SQL
                        if buf.len() >= BATCH_SIZE && tokio::time::Instant::now() >= next_retry
                            && !flush(&td, &mut buf).await
                        {
                            next_retry = tokio::time::Instant::now() + RETRY_INTERVAL;
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "drop unparseable event"),
                },
                Some(Err(e)) => tracing::warn!(error = %e, "kafka recv error"),
                None => break,
            },
            _ = ticker.tick() => {
                if !buf.is_empty() && !flush(&td, &mut buf).await {
                    next_retry = tokio::time::Instant::now() + RETRY_INTERVAL;
                }
            }
        }
    }
    // 流结束（订阅中断）时冲刷剩余
    if !buf.is_empty() {
        flush(&td, &mut buf).await;
    }
}

/// 有界缓冲：超过上限丢弃最旧事件并返回丢弃条数。
fn trim_buffer(buf: &mut Vec<EventMessage>) -> usize {
    let excess = buf.len().saturating_sub(MAX_BUFFER);
    if excess > 0 {
        buf.drain(..excess);
        tracing::warn!(
            dropped = excess,
            cap = MAX_BUFFER,
            "ingest buffer full, oldest events dropped"
        );
    }
    excess
}

/// 写成功才清空 buffer；失败保留数据由下一轮 tick/批次重试并返回 false。
async fn flush(td: &TdengineClient, buf: &mut Vec<EventMessage>) -> bool {
    let n = buf.len() as u64;
    let sql = batch_sql(buf);
    if let Err(e) = td.query(&sql).await {
        tracing::error!(error = %e, len = n, "tdengine batch write failed, buffer retained");
        crate::metrics::record_write_failure();
        return false;
    }
    buf.clear();
    crate::metrics::record_ingested(n);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(ts: i64) -> EventMessage {
        EventMessage {
            device_id: "d1".into(),
            tenant_id: "t1".into(),
            kind: "property".into(),
            code: "temp".into(),
            value: json!(1),
            ts,
        }
    }

    #[test]
    fn trim_buffer_drops_oldest_beyond_cap() {
        let mut buf: Vec<EventMessage> = (0..MAX_BUFFER as i64 + 3).map(ev).collect();
        assert_eq!(trim_buffer(&mut buf), 3);
        assert_eq!(buf.len(), MAX_BUFFER);
        assert_eq!(buf[0].ts, 3, "最旧的 3 条被丢弃");
        assert_eq!(buf[MAX_BUFFER - 1].ts, MAX_BUFFER as i64 + 2);
        assert_eq!(trim_buffer(&mut buf), 0);
    }
}
