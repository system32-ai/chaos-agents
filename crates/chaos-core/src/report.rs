use chrono::{DateTime, Utc};
use std::fmt;
use std::time::Duration;
use uuid::Uuid;

use crate::skill::TargetDomain;

/// Lightweight summary of a discovered resource.
#[derive(Debug, Clone)]
pub struct DiscoveredResourceSummary {
    pub resource_type: String,
    pub name: String,
}

/// Record of a single skill execution.
#[derive(Debug, Clone)]
pub struct SkillExecutionRecord {
    pub skill_name: String,
    pub success: bool,
    pub duration: Duration,
    pub error: Option<String>,
}

/// Record of a single rollback step.
#[derive(Debug, Clone)]
pub struct RollbackStepRecord {
    pub skill_name: String,
    pub success: bool,
    pub duration: Duration,
    pub error: Option<String>,
}

/// Complete post-experiment report.
#[derive(Debug, Clone)]
pub struct ExperimentReport {
    pub experiment_id: Uuid,
    pub experiment_name: String,
    pub target_domain: TargetDomain,
    pub status: String,

    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub total_duration: Duration,
    pub soak_duration: Duration,

    pub discovered_resources: Vec<DiscoveredResourceSummary>,
    pub skill_executions: Vec<SkillExecutionRecord>,
    pub rollback_steps: Vec<RollbackStepRecord>,
}

fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs >= 60 {
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        let millis = d.subsec_millis();
        if millis > 0 {
            format!("{mins}m {secs}.{millis:03}s")
        } else {
            format!("{mins}m {secs}s")
        }
    } else {
        let millis = d.as_millis();
        if millis < 1000 {
            format!("{millis}ms")
        } else {
            format!("{}.{}s", total_secs, d.subsec_millis() / 100)
        }
    }
}

impl fmt::Display for ExperimentReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bar = "=".repeat(72);
        let thin = "-".repeat(72);

        writeln!(f, "\n{bar}")?;
        writeln!(f, "  EXPERIMENT REPORT")?;
        writeln!(f, "{bar}\n")?;

        writeln!(f, "  Name:     {}", self.experiment_name)?;
        writeln!(f, "  ID:       {}", self.experiment_id)?;
        writeln!(f, "  Target:   {}", self.target_domain)?;
        writeln!(f, "  Status:   {}", self.status)?;
        writeln!(f, "  Duration: {}", format_duration(self.total_duration))?;

        // Discovery
        writeln!(f, "\n{thin}")?;
        writeln!(
            f,
            "  DISCOVERED RESOURCES ({})",
            self.discovered_resources.len()
        )?;
        writeln!(f, "{thin}\n")?;
        if self.discovered_resources.is_empty() {
            writeln!(f, "  (none)")?;
        } else {
            writeln!(f, "  {:<15} {}", "TYPE", "NAME")?;
            for r in &self.discovered_resources {
                writeln!(f, "  {:<15} {}", r.resource_type, r.name)?;
            }
        }

        // Skills executed
        writeln!(f, "\n{thin}")?;
        writeln!(
            f,
            "  SKILLS EXECUTED ({})",
            self.skill_executions.len()
        )?;
        writeln!(f, "{thin}\n")?;
        if self.skill_executions.is_empty() {
            writeln!(f, "  (none)")?;
        } else {
            writeln!(
                f,
                "  {:<4} {:<25} {:<10} {}",
                "#", "SKILL", "RESULT", "DURATION"
            )?;
            for (i, s) in self.skill_executions.iter().enumerate() {
                let result = if s.success { "OK" } else { "FAILED" };
                writeln!(
                    f,
                    "  {:<4} {:<25} {:<10} {}",
                    i + 1,
                    s.skill_name,
                    result,
                    format_duration(s.duration)
                )?;
                if let Some(ref err) = s.error {
                    writeln!(f, "       -> {err}")?;
                }
            }
        }

        // Rollback
        writeln!(f, "\n{thin}")?;
        writeln!(f, "  ROLLBACK ({} steps)", self.rollback_steps.len())?;
        writeln!(f, "{thin}\n")?;
        if self.rollback_steps.is_empty() {
            writeln!(f, "  (none)")?;
        } else {
            writeln!(
                f,
                "  {:<4} {:<25} {:<10} {}",
                "#", "SKILL", "RESULT", "DURATION"
            )?;
            for (i, r) in self.rollback_steps.iter().enumerate() {
                let result = if r.success { "OK" } else { "FAILED" };
                writeln!(
                    f,
                    "  {:<4} {:<25} {:<10} {}",
                    i + 1,
                    r.skill_name,
                    result,
                    format_duration(r.duration)
                )?;
                if let Some(ref err) = r.error {
                    writeln!(f, "       -> {err}")?;
                }
            }
        }

        // Timeline
        writeln!(f, "\n{thin}")?;
        writeln!(f, "  TIMELINE")?;
        writeln!(f, "{thin}\n")?;
        writeln!(
            f,
            "  Started:    {}",
            self.started_at.format("%Y-%m-%d %H:%M:%S UTC")
        )?;
        writeln!(
            f,
            "  Completed:  {}",
            self.completed_at.format("%Y-%m-%d %H:%M:%S UTC")
        )?;
        writeln!(
            f,
            "  Soak time:  {}",
            format_duration(self.soak_duration)
        )?;
        writeln!(
            f,
            "  Total:      {}",
            format_duration(self.total_duration)
        )?;

        writeln!(f, "\n{bar}")?;

        Ok(())
    }
}
