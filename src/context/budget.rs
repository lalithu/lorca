//! One snapshot shared by context fitting and the inference request. The actual
//! allocated window is the ceiling; this never switches models or enlarges it.
use serde::Serialize;
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationBudget {
    pub context_limit: u64,
    pub output_limit: u64,
    pub safety_margin: u64,
}
impl GenerationBudget {
    pub fn new(context_limit: u64, recovery: bool) -> Self {
        let initial = (context_limit / 8).clamp(256, 8192);
        let output_limit = if recovery {
            initial.saturating_mul(2).min(16384)
        } else {
            initial
        }
        .min(context_limit / 4);
        let safety_margin = (context_limit / 64).clamp(128, 1024).min(context_limit / 8);
        Self {
            context_limit,
            output_limit,
            safety_margin,
        }
    }
    pub fn scheduled(
        mut self,
        seconds: u64,
        usage: &crate::types::Usage,
        effort: crate::agent_types::ThinkingEffort,
    ) -> (Self, u64) {
        let speed = if usage.generated_tokens.unwrap_or(0) >= 32 {
            usage
                .tokens_per_second
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(40.0)
                .clamp(1.0, 200.0)
        } else {
            40.0
        };
        // An explicitly configured timeout remains a hard limit. Only the
        // default policy is adaptive; command and preparation limits are unchanged.
        let ceiling = if seconds != crate::agent_types::Limits::default().model_seconds {
            seconds
        } else {
            match effort {
                crate::agent_types::ThinkingEffort::Off => 180,
                crate::agent_types::ThinkingEffort::Nominal => 240,
                crate::agent_types::ThinkingEffort::High => 360,
            }
        };
        self.output_limit = self
            .output_limit
            .min(((ceiling.saturating_sub(15).max(1)) as f64 * speed) as u64);
        let timeout = seconds
            .max((self.output_limit as f64 / speed).ceil() as u64 + 15)
            .min(ceiling);
        (self, timeout)
    }
    pub fn input_limit(self) -> u64 {
        self.context_limit.saturating_sub(self.reserved())
    }
    pub fn reserved(self) -> u64 {
        self.output_limit.saturating_add(self.safety_margin)
    }
    pub fn check(self, input: u64) -> Result<(), String> {
        if self.output_limit == 0 || input > self.input_limit() {
            return Err(format!(
                "Context cannot fit essential input ({input} estimated tokens), output allowance ({}) and safety margin ({}) in the allocated {}-token window. Work is saved.",
                self.output_limit, self.safety_margin, self.context_limit
            ));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_scales_with_allocated_context_and_recovery_stays_bounded() {
        for (capacity, output) in [
            (4096, 512),
            (8192, 1024),
            (16384, 2048),
            (32768, 4096),
            (65536, 8192),
            (262144, 8192),
        ] {
            let normal = GenerationBudget::new(capacity, false);
            let retry = GenerationBudget::new(capacity, true);
            assert_eq!(normal.output_limit, output);
            assert_eq!(retry.output_limit, output * 2);
            assert_eq!(normal.input_limit() + normal.reserved(), capacity);
            assert!(retry.output_limit <= 16384);
            assert!(normal.check(normal.input_limit()).is_ok());
            assert!(normal.check(normal.input_limit() + 1).is_err());
        }
    }
    #[test]
    fn even_unknown_or_tiny_windows_never_overcommit() {
        for capacity in [0, 1, 16, 128, 1024, u64::MAX] {
            let budget = GenerationBudget::new(capacity, true);
            assert!(budget.reserved() <= capacity);
            if capacity < 4 {
                assert!(budget.check(1).is_err());
            }
        }
    }
}

#[cfg(test)]
mod scheduling_tests {
    use super::*;
    use crate::{agent_types::ThinkingEffort, types::Usage};
    #[test]
    fn time_and_output_are_coordinated_without_enlarging_the_context() {
        for effort in [
            ThinkingEffort::Off,
            ThinkingEffort::Nominal,
            ThinkingEffort::High,
        ] {
            let usage = Usage {
                generated_tokens: Some(100),
                tokens_per_second: Some(20.0),
                ..Default::default()
            };
            let base = GenerationBudget::new(32768, true);
            let (budget, seconds) = base.scheduled(120, &usage, effort);
            assert_eq!(budget.context_limit, 32768);
            assert!(budget.output_limit <= base.output_limit);
            assert!(seconds >= (budget.output_limit as f64 / 20.0).ceil() as u64 + 15);
            assert!(seconds <= 360);
        }
    }
    #[test]
    fn empty_speed_and_impossible_usage_cannot_inflate_budgets() {
        for speed in [f64::NAN, f64::INFINITY, -1.0, 1_000_000.0] {
            let usage = Usage {
                generated_tokens: Some(1),
                tokens_per_second: Some(speed),
                ..Default::default()
            };
            let (a, seconds) =
                GenerationBudget::new(8192, false).scheduled(120, &usage, ThinkingEffort::Nominal);
            assert_eq!(a.output_limit, 1024);
            assert_eq!(seconds, 120);
        }
        let (zero, _) =
            GenerationBudget::new(0, false).scheduled(120, &Usage::default(), ThinkingEffort::High);
        assert_eq!(zero.output_limit, 0);
    }
}

#[cfg(test)]
mod explicit_timeout_tests {
    use super::*;
    #[test]
    fn an_explicit_timeout_is_never_overridden_by_effort() {
        for seconds in [1, 30, 90, 180] {
            for effort in [
                crate::agent_types::ThinkingEffort::Off,
                crate::agent_types::ThinkingEffort::Nominal,
                crate::agent_types::ThinkingEffort::High,
            ] {
                let (budget, timeout) = GenerationBudget::new(32768, true).scheduled(
                    seconds,
                    &crate::types::Usage::default(),
                    effort,
                );
                assert_eq!(timeout, seconds);
                assert!(budget.output_limit <= 8192);
            }
        }
    }
}
