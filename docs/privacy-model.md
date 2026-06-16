# veyo — privacy model

> **This is the moat.** The reduction that makes `veyo` cheap is the *same act* that makes it private: once a frame becomes a short text delta on-device, there is nothing pixel-shaped left to leak. This document states the invariants, shows how the type system enforces them, and lays out the compliance story.

The guiding principle: **what crosses any process or network boundary is text describing change, never pixels.**

---

## 1. Privacy invariants (non-negotiable)

These five are not features to be balanced against others — they are the product's reason to exist.

1. **Raw frames live only in an in-memory ring buffer; never persisted to disk by default.**
2. **Nothing pixel-based crosses the process boundary — only structured deltas.**
3. **`evidence` references stay local.** A remote consumer can ask `describe_region()` and get *text*, never the image.
4. **Tier-2 enrichment (OCR/UI captioning) runs on-device.** If summaries may contain PII and a *cloud* LLM is the consumer, route summaries through **CloakPipe** (text-layer) before emit. Fully-local consumers skip this.
5. **A bi-temporal audit log of exactly what was emitted = the DPDP/compliance evidence story.**

---

## 2. The reduction *is* the guarantee

Most "private" capture tools record everything raw and then try to protect it — encryption at rest, access controls, redaction passes. `veyo` inverts this: the sensitive artifact (the frame) is **destroyed by design** as part of normal operation. A frame enters the in-memory ring buffer, is reduced to a perceptual hash and a short summary, and is overwritten. There is no raw archive to breach, subpoena, or exfiltrate.

This is why the cost story and the privacy story are the same sentence: *visual data can live as cheap text deltas at the edge.* You don't pay for privacy with a worse product; the cheap path and the private path are one path.

---

## 3. Enforced by the type system, not by discipline

Invariant #2 is the easiest to violate by accident — one careless `serde::Serialize` and a thumbnail rides out on the wire. So it is enforced in the **types**, not in code review:

- The `Delta.evidence` field carries a `#[serde(skip)]`-style boundary, so it **physically cannot be serialized** onto the MCP wire. (See the type in [Tech Specs](tech-specs.md#core-data-model).)
- `evidence` holds only a perceptual hash (`phash`) and a local cache reference (`thumb_ref`) — both **local-only by construction**. They exist for local audit/replay and lazy on-demand enrichment, never for export.

If a future change tried to emit pixels, it would have to *deliberately* defeat the type boundary — which is reviewable in a way that "remembering not to log the frame" never is.

---

## 4. The `describe_region` escape hatch

A remote consumer sometimes legitimately needs *more* than the templated summary — "what did that dialog actually say?". The answer is `describe_region(region_id)`:

- It runs Tier-2 enrichment **on-device**, on that one region's local thumbnail.
- It returns **text only** — never the image.

So the system can be more informative on demand without ever weakening invariant #2. Detail is pulled as *description*, not as pixels. The tool is defined in [Architecture §5](architecture.md#mcp-surface).

---

## 5. Cloud consumers & CloakPipe

The default posture assumes a **fully-local** consumer (a local LLM / agent), for which no text ever leaves the device and invariants #1–#3 fully cover the privacy story.

When the consumer is a **cloud** LLM, the summaries themselves — though text, not pixels — may contain PII (a visible email, an account number that OCR lifted). For that case:

- Route summaries through **CloakPipe** (the text-layer PII gate) before emit.
- This extends CloakPipe's existing DPDP evidence story into the *visual* modality — the same redaction guarantees, now covering what `veyo` sees.

CloakPipe routing is **out of v1 scope** (see [Phases](phases.md)) — the interface is anticipated, the integration is later.

---

## 6. Compliance & audit story {#compliance}

The **bi-temporal log** is not just a memory feature — it is the compliance artifact. Because every delta records both `t_event` (when it happened on screen) and `t_observed` (when the codec emitted it), the store is a complete, timestamped record of **exactly what information left the sensor and when**.

- For a regulator's question — *"what did this system observe and transmit about the user?"* — the answer is a query, not a forensic reconstruction.
- The log contains **text deltas only**; there is no raw-frame archive to produce, which is itself the strongest possible answer to *"what could have leaked?"*.
- This is the DPDP (and adjacent regimes') evidence posture that the commercial/compliance tier is built on (see [Product Overview §7](product-overview.md#7-licensing)).

---

## 7. Summary table

| Boundary | What can cross | What cannot |
|---|---|---|
| Ring buffer → anything | reduced hashes, summaries | raw frames (overwritten in memory) |
| Process → MCP wire | structured deltas (text) | `evidence` (type-fenced), thumbnails, pixels |
| Device → cloud LLM | summaries, optionally CloakPipe-sanitized | imagery, `evidence`, raw frames |
| `describe_region` response | text description | the region image |

---

## Related

- The fenced type that enforces invariant #2: **[Tech Specs](tech-specs.md#core-data-model)**.
- Where `describe_region` lives: **[Architecture](architecture.md#mcp-surface)**.
- The eval harness must verify nothing pixel-shaped is ever emitted: **[Eval Harness](eval-harness.md)**.
- Schema-freeze as a stability/compliance guarantee: **[Risks & Open Questions](risks-and-open-questions.md)**.
