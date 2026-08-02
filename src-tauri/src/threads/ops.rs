//! Den aegte `mcp::ThreadOps`. Bor i lib'en (ikke i main.rs), fordi
//! argument-oversaettelse og svarform er beslutninger.

use serde_json::{json, Value};

use super::{pair, FromKind, Intent, PostRequest};

pub struct LiveThreadOps;

fn intent_from_wire(raw: &str) -> Result<Intent, String> {
    match raw {
        "sparring" => Ok(Intent::Sparring),
        "delegation" => Ok(Intent::Delegation),
        "answer" => Ok(Intent::Answer),
        "status" => Ok(Intent::Status),
        other => Err(format!(
            "unknown intent: {other} - valid values are sparring, delegation, answer, status"
        )),
    }
}

impl crate::mcp::ThreadOps for LiveThreadOps {
    fn pair(
        &self,
        from_card: &str,
        agent: &str,
        purpose: &str,
        opening: &str,
    ) -> Result<Value, String> {
        let r = pair::card_pair(from_card, agent, purpose, opening)?;
        Ok(json!({ "thread": r.thread, "partner": r.partner, "chat_card": r.chat_card }))
    }

    fn say(
        &self,
        from_card: &str,
        thread: &str,
        text: &str,
        intent: &str,
    ) -> Result<Value, String> {
        let accepted = super::post(PostRequest {
            thread: thread.to_string(),
            from_card: from_card.to_string(),
            // Arten saettes af BACKENDEN, aldrig af modellen (spec §3.1).
            from_kind: FromKind::Agent,
            intent: intent_from_wire(intent)?,
            text: text.to_string(),
        })?;
        Ok(json!({
            "accepted": true,
            "seq": accepted.seq,
            "hop": accepted.hop,
            "hops_left": accepted.hops_left
        }))
    }

    fn inbox(&self, card: &str, thread: &str, ack_through: Option<u64>) -> Result<Value, String> {
        let batch = super::inbox_take(thread, card, ack_through)?;
        Ok(json!({
            "messages": batch.messages.iter().map(|m| json!({
                "seq": m.seq,
                "from_card": m.from_card,
                "from_kind": m.from_kind.as_str(),
                "intent": m.intent.as_str(),
                "text": m.text,
                "ts_ms": m.ts_ms,
            })).collect::<Vec<_>>(),
            "has_more": batch.has_more,
            "batch_id": batch.batch_id
        }))
    }
}
