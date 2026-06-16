//! One-page text report for a scored session.

use crate::score::Scored;

/// Render a one-page report for `name` from its [`Scored`] result.
pub fn render(name: &str, s: &Scored) -> String {
    let verdict = if s.annotated == 0 || s.frames == 0 {
        "N/A — unscoreable (no annotations or no frames)"
    } else if s.passes_gate() {
        "PASS — recall ≥ 0.9 at emission < 1%"
    } else if s.recall < 0.9 {
        "below gate — recall too low"
    } else {
        "below gate — emission rate too high"
    };
    format!(
        "veyo-eval — session: {name}\n\
         ----------------------------------------\n\
         frames          {frames}\n\
         annotations     {annotated}\n\
         emissions       {emitted}\n\
         matched         {matched}\n\
         recall          {recall:.3}    (gate \u{2265} 0.900)\n\
         precision       {precision:.3}\n\
         emission rate   {erate:.4}   (gate < 0.0100)\n\
         events/hour     {eph:.1}\n\
         ----------------------------------------\n\
         verdict         {verdict}\n",
        name = name,
        frames = s.frames,
        annotated = s.annotated,
        emitted = s.emitted,
        matched = s.matched,
        recall = s.recall,
        precision = s.precision,
        erate = s.emission_rate,
        eph = s.events_per_hour,
        verdict = verdict,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scored(recall: f32, erate: f32) -> Scored {
        Scored {
            annotated: 1,
            emitted: 2,
            matched: 1,
            frames: 100,
            duration_ms: 1000,
            recall,
            precision: 0.5,
            emission_rate: erate,
            events_per_hour: 7200.0,
        }
    }

    #[test]
    fn report_includes_name_recall_and_verdict() {
        let r = render("demo", &scored(1.0, 0.005));
        assert!(r.contains("demo"));
        assert!(r.contains("recall"));
        assert!(r.contains("PASS"));
    }

    #[test]
    fn low_recall_reports_below_gate() {
        let r = render("demo", &scored(0.5, 0.005));
        assert!(r.contains("below gate"));
    }

    #[test]
    fn unscoreable_session_reports_na_not_pass() {
        let empty = Scored {
            annotated: 0,
            emitted: 0,
            matched: 0,
            frames: 0,
            duration_ms: 0,
            recall: 1.0,
            precision: 1.0,
            emission_rate: 0.0,
            events_per_hour: 0.0,
        };
        let r = render("empty", &empty);
        assert!(r.contains("N/A"), "got: {r}");
        assert!(!r.contains("PASS"));
    }
}
