//! Workflows (phase 7, D§5.6, D§9.7; decision: workflows): runs driven
//! from job reports through the event-log consumer, routing, joins, loops,
//! the result contract, reconciliation, triggers, artifacts, REST and MCP.
//! Clock-injected; the consumer is drained directly.

mod common;

use axum::http::StatusCode;
use serde_json::{json, Value};
use tower_core::{RunState, StepState, Task, TaskState, TowerError, WorkflowRun, WorkflowStep};
use tower_server::{schedules, tasks, workflows};

const T0: i64 = 1_000_000;

const ISSUE: &str = r#"
name = "issue"
result = "review"

[inputs]
issue = { required = true }

[[nodes]]
id = "triage"
agent = "triager"
outcomes = ["build", "design"]
prompt = "Read issue {{ input.issue }}."

[[nodes]]
id = "work"
agent = { from = "triage", map = { build = "builder", design = "designer" } }
context = ["triage", "review"]
max_visits = 3
prompt = "Do the work for {{ input.issue }}."

[[nodes]]
id = "review"
agent = "reviewer"
outcomes = ["approved", "changes"]
prompt = "Review {{ input.issue }}."

[[edges]]
from = "triage"
to = "work"

[[edges]]
from = "work"
to = "review"

[[edges]]
from = "review"
to = "work"
on = "changes"
"#;

const DIGEST: &str = r#"
name = "digest"

[[nodes]]
id = "slack"
agent = "slack-scout"
prompt = "Collect from Slack."

[[nodes]]
id = "linear"
agent = "linear-scout"
prompt = "Collect from Linear."

[[nodes]]
id = "aggregate"
agent = "librarian"
join = "all"
context = ["slack", "linear"]
prompt = "Merge both."

[[edges]]
from = "slack"
to = "aggregate"

[[edges]]
from = "linear"
to = "aggregate"
"#;

const ISSUE_AGENTS: &[&str] = &["triager", "builder", "designer", "reviewer"];

async fn boot_with(agents: &[&str]) -> common::Ctx {
    let ctx = common::boot().await;
    for a in agents {
        ctx.spawn(a, "omp").await;
    }
    ctx
}

async fn apply(ctx: &common::Ctx, src: &str) -> workflows::Applied {
    workflows::apply(&ctx.state, src, T0).await.unwrap()
}

async fn start(ctx: &common::Ctx, name: &str, input: Value) -> WorkflowRun {
    workflows::start_run(
        &ctx.state,
        name,
        input.as_object().cloned().unwrap_or_default(),
        json!({ "kind": "manual" }),
        T0,
    )
    .await
    .unwrap()
}

async fn steps(ctx: &common::Ctx, run: &WorkflowRun) -> Vec<WorkflowStep> {
    let v = workflows::show_run(&ctx.state, &run.id).await.unwrap();
    serde_json::from_value(v["steps"].clone()).unwrap()
}

async fn reload(ctx: &common::Ctx, run: &WorkflowRun) -> WorkflowRun {
    workflows::get_run(&ctx.state, &run.id)
        .await
        .unwrap()
        .unwrap()
}

/// The newest open step of `node`.
async fn open_step(ctx: &common::Ctx, run: &WorkflowRun, node: &str) -> WorkflowStep {
    steps(ctx, run)
        .await
        .into_iter()
        .filter(|s| s.node_id == node && s.state.is_open())
        .max_by_key(|s| s.visit)
        .unwrap_or_else(|| panic!("no open step for `{node}`"))
}

async fn job(ctx: &common::Ctx, step: &WorkflowStep) -> Task {
    tasks::get_task(&ctx.state, step.task_id.as_ref().unwrap())
        .await
        .unwrap()
        .expect("the step's job exists")
}

async fn owner_name(ctx: &common::Ctx, t: &Task) -> String {
    let id = t.owner_id.as_ref().expect("job delivered");
    tower_server::inventory::get_agent(&ctx.state, &id.0)
        .await
        .unwrap()
        .unwrap()
        .name
}

/// Report as the job's owner, then let the engine follow the log.
async fn report(ctx: &common::Ctx, step: &WorkflowStep, state: TaskState, result: Value) {
    let t = job(ctx, step).await;
    let owner = owner_name(ctx, &t).await;
    tasks::report(&ctx.state, &t.id, &owner, Some(state), Some(result), T0)
        .await
        .unwrap();
    workflows::drain(&ctx.state, T0).await.unwrap();
}

async fn complete(ctx: &common::Ctx, step: &WorkflowStep, result: Value) {
    report(ctx, step, TaskState::Completed, result).await;
}

fn reason(e: &anyhow::Error) -> String {
    let err = e.downcast_ref::<TowerError>().expect("TowerError");
    err.detail.as_ref().expect("detail")["reason"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn count_runs(ctx: &common::Ctx, workflow: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM workflow_runs WHERE workflow_name = ?1")
        .bind(workflow)
        .fetch_one(&ctx.state.pool)
        .await
        .unwrap()
}

// ---- routing, loops, hand-off ----------------------------------------------

#[tokio::test]
async fn issue_run_branches_on_the_outcome_and_loops_until_review_approves() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    apply(&ctx, ISSUE).await;
    let run = start(&ctx, "issue", json!({ "issue": "ENG-1" })).await;

    let triage = open_step(&ctx, &run, "triage").await;
    let t = job(&ctx, &triage).await;
    assert_eq!(t.origin, "workflow");
    assert_eq!(t.context_id.as_deref(), Some(run.id.0.as_str()));
    assert_eq!(owner_name(&ctx, &t).await, "triager", "delivered at once");
    let d = t.description.unwrap();
    assert!(d.starts_with("Read issue ENG-1."), "{d}");
    assert!(
        d.contains("`outcome` is required: one of `build`, `design`"),
        "{d}"
    );

    complete(
        &ctx,
        &triage,
        json!({ "summary": "it is UI work", "outcome": "design" }),
    )
    .await;
    let work = open_step(&ctx, &run, "work").await;
    let t = job(&ctx, &work).await;
    assert_eq!(
        owner_name(&ctx, &t).await,
        "designer",
        "agent follows the outcome"
    );
    let d = t.description.unwrap();
    assert!(
        d.contains("`triage` visit 1 → outcome `design`\n  it is UI work"),
        "{d}"
    );

    complete(&ctx, &work, json!("drew the screens")).await;
    let review = open_step(&ctx, &run, "review").await;
    assert!(job(&ctx, &review)
        .await
        .description
        .unwrap()
        .contains("drew the screens"));
    complete(
        &ctx,
        &review,
        json!({ "summary": "fix the colors", "outcome": "changes" }),
    )
    .await;

    let work2 = open_step(&ctx, &run, "work").await;
    assert_eq!(work2.visit, 2);
    let t = job(&ctx, &work2).await;
    assert_eq!(owner_name(&ctx, &t).await, "designer");
    let d = t.description.unwrap();
    assert!(d.contains("visit 2 of at most 3"), "{d}");
    assert!(
        d.contains("fix the colors"),
        "the review reaches the worker: {d}"
    );
    assert!(
        d.contains("`work` visit 1 (this step, earlier)\n  drew the screens"),
        "{d}"
    );

    complete(&ctx, &work2, json!("fixed the colors")).await;
    let review2 = open_step(&ctx, &run, "review").await;
    complete(
        &ctx,
        &review2,
        json!({ "summary": "ship it", "outcome": "approved" }),
    )
    .await;

    let done = reload(&ctx, &run).await;
    assert_eq!(done.state, RunState::Completed);
    let result = done.result.unwrap();
    assert_eq!(result["node"], "review");
    assert_eq!(result["outcome"], "approved");
    assert_eq!(result["summary"], "ship it");
    let all = steps(&ctx, &run).await;
    assert_eq!(all.len(), 5);
    assert!(all.iter().all(|s| s.state == StepState::Completed));
}

#[tokio::test]
async fn a_loop_past_max_visits_fails_its_step_and_the_run() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    apply(&ctx, &ISSUE.replace("max_visits = 3", "max_visits = 2")).await;
    let run = start(&ctx, "issue", json!({ "issue": "ENG-2" })).await;
    let triage = open_step(&ctx, &run, "triage").await;
    complete(&ctx, &triage, json!({ "summary": "s", "outcome": "build" })).await;
    for _ in 0..2 {
        let work = open_step(&ctx, &run, "work").await;
        complete(&ctx, &work, json!("done")).await;
        let review = open_step(&ctx, &run, "review").await;
        complete(
            &ctx,
            &review,
            json!({ "summary": "again", "outcome": "changes" }),
        )
        .await;
    }
    let done = reload(&ctx, &run).await;
    assert_eq!(done.state, RunState::Failed);
    assert!(
        done.error
            .as_deref()
            .unwrap()
            .contains("max_visits (2) reached"),
        "{:?}",
        done.error
    );
    let third = steps(&ctx, &run)
        .await
        .into_iter()
        .find(|s| s.node_id == "work" && s.visit == 3)
        .unwrap();
    assert_eq!(third.state, StepState::Failed);
    assert_eq!(
        tasks::get_task(&ctx.state, third.task_id.as_ref().unwrap())
            .await
            .unwrap()
            .map(|t| t.id),
        None,
        "an over-limit visit never creates a job"
    );
}

#[tokio::test]
async fn a_workflow_jobs_completion_must_meet_the_result_contract() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    apply(&ctx, ISSUE).await;
    let run = start(&ctx, "issue", json!({ "issue": "ENG-3" })).await;
    let triage = open_step(&ctx, &run, "triage").await;
    let t = job(&ctx, &triage).await;
    let try_report = |result: Value| {
        let state = ctx.state.clone();
        let id = t.id.clone();
        async move {
            tasks::report(
                &state,
                &id,
                "triager",
                Some(TaskState::Completed),
                Some(result),
                T0,
            )
            .await
        }
    };
    for (result, want) in [
        (json!({ "outcome": "build" }), "summary_required"),
        (
            json!({ "summary": "  ", "outcome": "build" }),
            "summary_required",
        ),
        (json!({ "summary": "x" }), "outcome_invalid"),
        (
            json!({ "summary": "x", "outcome": "ops" }),
            "outcome_invalid",
        ),
        (
            json!({ "summary": "x".repeat(2001), "outcome": "build" }),
            "summary_too_long",
        ),
        (
            json!({ "summary": "x", "outcome": "build", "data": "y".repeat(17 * 1024) }),
            "data_too_large",
        ),
        (json!(42), "summary_required"),
    ] {
        let e = try_report(result.clone()).await.unwrap_err();
        assert_eq!(reason(&e), want, "{result}");
    }
    assert_eq!(
        job(&ctx, &triage).await.state,
        TaskState::Assigned,
        "still open"
    );
    let ok = try_report(json!({ "summary": "x".repeat(2000), "outcome": "build" }))
        .await
        .unwrap();
    assert_eq!(ok.state, TaskState::Completed);

    // a non-owner gets the job's conflict, not the contract
    let work = {
        workflows::drain(&ctx.state, T0).await.unwrap();
        open_step(&ctx, &run, "work").await
    };
    let e = tasks::report(
        &ctx.state,
        work.task_id.as_ref().unwrap(),
        "reviewer",
        Some(TaskState::Completed),
        None,
        T0,
    )
    .await
    .unwrap_err();
    assert_eq!(
        e.downcast_ref::<TowerError>().unwrap().code,
        tower_core::ErrorCode::Conflict
    );

    // jobs outside workflows are unchanged
    let plain = tasks::create(
        &ctx.state,
        tasks::CreateTask {
            title: "plain".into(),
            description: None,
            priority: None,
            tags: vec![],
            assign: Some("reviewer".into()),
            when_available: false,
            not_before: None,
            lease_s: None,
            max_attempts: None,
            pre_run: None,
            post_run: None,
        },
        T0,
    )
    .await
    .unwrap();
    tasks::report(
        &ctx.state,
        &plain.id,
        "reviewer",
        Some(TaskState::Completed),
        None,
        T0,
    )
    .await
    .unwrap();
}

// ---- joins -----------------------------------------------------------------

#[tokio::test]
async fn fan_out_then_join_all_runs_the_join_once_with_both_summaries() {
    let ctx = boot_with(&["slack-scout", "linear-scout", "librarian"]).await;
    apply(&ctx, DIGEST).await;
    let run = start(&ctx, "digest", json!({})).await;
    let slack = open_step(&ctx, &run, "slack").await;
    let linear = open_step(&ctx, &run, "linear").await;
    assert_eq!(slack.state, StepState::Running);
    assert_eq!(linear.state, StepState::Running);

    complete(&ctx, &slack, json!("three decisions in #eng")).await;
    let pending = open_step(&ctx, &run, "aggregate").await;
    assert_eq!(pending.state, StepState::Pending, "waits for linear");
    assert!(
        tasks::get_task(&ctx.state, pending.task_id.as_ref().unwrap())
            .await
            .unwrap()
            .is_none(),
        "no job while collecting"
    );

    complete(&ctx, &linear, json!("two projects moved")).await;
    let agg = open_step(&ctx, &run, "aggregate").await;
    assert_eq!(agg.state, StepState::Running);
    assert_eq!(agg.activated_by.len(), 2);
    let d = job(&ctx, &agg).await.description.unwrap();
    assert!(
        d.contains("three decisions in #eng") && d.contains("two projects moved"),
        "{d}"
    );

    complete(&ctx, &agg, json!("merged")).await;
    assert_eq!(reload(&ctx, &run).await.state, RunState::Completed);
    let aggregates = steps(&ctx, &run)
        .await
        .into_iter()
        .filter(|s| s.node_id == "aggregate")
        .count();
    assert_eq!(aggregates, 1);
}

#[tokio::test]
async fn a_join_waiting_on_an_untaken_branch_runs_when_the_run_goes_quiet() {
    let ctx = boot_with(&["x"]).await;
    apply(
        &ctx,
        r#"
name = "branchy"
[[nodes]]
id = "a"
agent = "x"
outcomes = ["left", "right"]
prompt = "choose"
[[nodes]]
id = "b"
agent = "x"
prompt = "left work"
[[nodes]]
id = "c"
agent = "x"
prompt = "right work"
[[nodes]]
id = "d"
agent = "x"
join = "all"
prompt = "finish"
[[edges]]
from = "a"
to = "b"
on = "left"
[[edges]]
from = "a"
to = "c"
on = "right"
[[edges]]
from = "b"
to = "d"
[[edges]]
from = "c"
to = "d"
"#,
    )
    .await;
    let run = start(&ctx, "branchy", json!({})).await;
    complete(
        &ctx,
        &open_step(&ctx, &run, "a").await,
        json!({ "summary": "go", "outcome": "left" }),
    )
    .await;
    let b = open_step(&ctx, &run, "b").await;
    complete(&ctx, &b, json!("left done")).await;
    let d = open_step(&ctx, &run, "d").await;
    assert_eq!(
        d.state,
        StepState::Running,
        "c never fires; the quiet run starts d"
    );
    assert_eq!(d.activated_by, vec![b.id.clone()]);
    complete(&ctx, &d, json!("finished")).await;
    assert_eq!(reload(&ctx, &run).await.state, RunState::Completed);
    assert!(steps(&ctx, &run).await.iter().all(|s| s.node_id != "c"));
}

// ---- failure ---------------------------------------------------------------

#[tokio::test]
async fn a_failed_edge_routes_and_an_unhandled_failure_cancels_the_siblings() {
    let ctx = boot_with(&["x", "y", "z"]).await;
    apply(
        &ctx,
        r#"
name = "recover"
[[nodes]]
id = "a"
agent = "x"
prompt = "try"
[[nodes]]
id = "fix"
agent = "y"
prompt = "fix it"
[[edges]]
from = "a"
to = "fix"
on = "failed"
"#,
    )
    .await;
    let run = start(&ctx, "recover", json!({})).await;
    report(
        &ctx,
        &open_step(&ctx, &run, "a").await,
        TaskState::Failed,
        json!({ "error": "disk full" }),
    )
    .await;
    let fix = open_step(&ctx, &run, "fix").await;
    let d = job(&ctx, &fix).await.description.unwrap();
    assert!(
        d.contains("`a` visit 1 → failed: job failed: disk full"),
        "{d}"
    );
    complete(&ctx, &fix, json!("freed space")).await;
    assert_eq!(reload(&ctx, &run).await.state, RunState::Completed);

    apply(
        &ctx,
        r#"
name = "pair"
[[nodes]]
id = "p"
agent = "x"
prompt = "one"
[[nodes]]
id = "q"
agent = "z"
prompt = "two"
"#,
    )
    .await;
    let run = start(&ctx, "pair", json!({})).await;
    let q = open_step(&ctx, &run, "q").await;
    report(
        &ctx,
        &open_step(&ctx, &run, "p").await,
        TaskState::Failed,
        json!("nope"),
    )
    .await;
    let failed = reload(&ctx, &run).await;
    assert_eq!(failed.state, RunState::Failed);
    assert!(failed.error.unwrap().contains("step `p` (visit 1) failed"));
    let q_now = steps(&ctx, &run)
        .await
        .into_iter()
        .find(|s| s.id == q.id)
        .unwrap();
    assert_eq!(q_now.state, StepState::Canceled);
    assert_eq!(job(&ctx, &q).await.state, TaskState::Canceled);
}

#[tokio::test]
async fn an_operator_cancel_of_a_step_job_fails_the_step() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    apply(&ctx, ISSUE).await;
    let run = start(&ctx, "issue", json!({ "issue": "ENG-4" })).await;
    let triage = open_step(&ctx, &run, "triage").await;
    tasks::cancel(&ctx.state, triage.task_id.as_ref().unwrap(), T0)
        .await
        .unwrap();
    workflows::drain(&ctx.state, T0).await.unwrap();
    let done = reload(&ctx, &run).await;
    assert_eq!(done.state, RunState::Failed);
    assert!(done.error.unwrap().contains("job canceled"));
}

#[tokio::test]
async fn an_unmapped_outcome_fails_the_step_that_needs_an_agent() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    apply(&ctx, &ISSUE.replace(", design = \"designer\"", "")).await;
    let run = start(&ctx, "issue", json!({ "issue": "ENG-5" })).await;
    let triage = open_step(&ctx, &run, "triage").await;
    complete(
        &ctx,
        &triage,
        json!({ "summary": "s", "outcome": "design" }),
    )
    .await;
    let done = reload(&ctx, &run).await;
    assert_eq!(done.state, RunState::Failed);
    assert!(done
        .error
        .unwrap()
        .contains("no agent mapped for outcome `design`"));
}

#[tokio::test]
async fn cancel_run_cancels_its_open_jobs_once() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    apply(&ctx, ISSUE).await;
    let run = start(&ctx, "issue", json!({ "issue": "ENG-6" })).await;
    let triage = open_step(&ctx, &run, "triage").await;
    let canceled = workflows::cancel_run(&ctx.state, &run.id, T0)
        .await
        .unwrap();
    assert_eq!(canceled.state, RunState::Canceled);
    assert_eq!(job(&ctx, &triage).await.state, TaskState::Canceled);
    workflows::drain(&ctx.state, T0).await.unwrap();
    let s = steps(&ctx, &run).await;
    assert_eq!(
        s[0].state,
        StepState::Canceled,
        "the cancel event changes nothing"
    );
    let e = workflows::cancel_run(&ctx.state, &run.id, T0)
        .await
        .unwrap_err();
    assert_eq!(
        e.downcast_ref::<TowerError>().unwrap().code,
        tower_core::ErrorCode::Conflict
    );
}

// ---- reconciliation --------------------------------------------------------

#[tokio::test]
async fn a_lost_completion_event_is_recovered_by_reconcile() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    apply(&ctx, ISSUE).await;
    let run = start(&ctx, "issue", json!({ "issue": "ENG-7" })).await;
    let triage = open_step(&ctx, &run, "triage").await;
    workflows::drain(&ctx.state, T0).await.unwrap();
    tasks::report(
        &ctx.state,
        triage.task_id.as_ref().unwrap(),
        "triager",
        Some(TaskState::Completed),
        Some(json!({ "summary": "s", "outcome": "build" })),
        T0,
    )
    .await
    .unwrap();
    // the events are lost: the consumer's cursor jumps past them
    sqlx::query("UPDATE event_consumers SET cursor = (SELECT MAX(seq) FROM events)")
        .execute(&ctx.state.pool)
        .await
        .unwrap();
    workflows::drain(&ctx.state, T0).await.unwrap();
    assert_eq!(
        open_step(&ctx, &run, "triage").await.state,
        StepState::Running
    );

    assert!(workflows::reconcile(&ctx.state, T0).await.unwrap() >= 1);
    let work = open_step(&ctx, &run, "work").await;
    assert_eq!(owner_name(&ctx, &job(&ctx, &work).await).await, "builder");
}

#[tokio::test]
async fn a_ready_step_gets_exactly_one_job_across_reconciles() {
    let ctx = boot_with(&["x"]).await;
    apply(
        &ctx,
        "name = \"one\"\n[[nodes]]\nid = \"a\"\nagent = \"x\"\nprompt = \"p\"\n",
    )
    .await;
    let run = start(&ctx, "one", json!({})).await;
    let a = open_step(&ctx, &run, "a").await;
    let task = a.task_id.clone().unwrap();

    // crash after the job insert, before the step update
    sqlx::query("UPDATE workflow_steps SET state = 'ready' WHERE id = ?1")
        .bind(a.id.0.as_str())
        .execute(&ctx.state.pool)
        .await
        .unwrap();
    workflows::reconcile(&ctx.state, T0).await.unwrap();
    workflows::reconcile(&ctx.state, T0).await.unwrap();
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE context_id = ?1")
        .bind(run.id.0.as_str())
        .fetch_one(&ctx.state.pool)
        .await
        .unwrap();
    assert_eq!(jobs, 1);
    assert_eq!(open_step(&ctx, &run, "a").await.state, StepState::Running);

    // crash after the step insert, before the job insert
    for sql in [
        "DELETE FROM messages WHERE task_id = ?1",
        "DELETE FROM tasks WHERE id = ?1",
    ] {
        sqlx::query(sql)
            .bind(task.0.as_str())
            .execute(&ctx.state.pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE workflow_steps SET state = 'ready' WHERE id = ?1")
        .bind(a.id.0.as_str())
        .execute(&ctx.state.pool)
        .await
        .unwrap();
    workflows::reconcile(&ctx.state, T0).await.unwrap();
    let t = tasks::get_task(&ctx.state, &task)
        .await
        .unwrap()
        .expect("recreated");
    assert_eq!(t.state, TaskState::Assigned);
    assert_eq!(open_step(&ctx, &run, "a").await.state, StepState::Running);
}

// ---- triggers --------------------------------------------------------------

#[tokio::test]
async fn a_schedule_trigger_starts_runs_once_per_occurrence_and_skips_while_open() {
    let ctx = boot_with(&["slack-scout", "linear-scout", "librarian"]).await;
    let src = format!(
        "{DIGEST}\n[[triggers]]\nkind = \"schedule\"\ncron = \"*/5 * * * *\"\ntimezone = \"UTC\"\n"
    );
    let applied = apply(&ctx, &src).await;
    let managed = schedules::list(&ctx.state).await.unwrap();
    assert_eq!(managed.len(), 1);
    let s = &managed[0];
    assert_eq!(s.workflow_id.as_ref(), Some(&applied.workflow.id));
    let first = s.next_run_at.unwrap();

    assert_eq!(schedules::fire_due(&ctx.state, first).await.unwrap(), 1);
    assert_eq!(
        schedules::fire_due(&ctx.state, first).await.unwrap(),
        0,
        "exactly once"
    );
    assert_eq!(count_runs(&ctx, "digest").await, 1);
    let run = &workflows::list_runs(&ctx.state, &Default::default())
        .await
        .unwrap()[0];
    assert_eq!(run.trigger["kind"], "schedule");
    assert_eq!(run.trigger["schedule_id"], s.id.0.as_str());

    let second = first + 5 * 60_000;
    assert_eq!(schedules::fire_due(&ctx.state, second).await.unwrap(), 1);
    assert_eq!(
        count_runs(&ctx, "digest").await,
        1,
        "skipped while the run is open"
    );
    assert_eq!(ctx.event_count("schedule.skipped").await, 1);

    // re-apply unchanged keeps the row; a new cadence replaces it
    apply(&ctx, &src).await;
    assert_eq!(schedules::list(&ctx.state).await.unwrap()[0].id, s.id);
    apply(&ctx, &src.replace("*/5", "*/7")).await;
    let now = schedules::list(&ctx.state).await.unwrap();
    assert_eq!(now.len(), 1);
    assert_ne!(now[0].id, s.id);

    // the definition owns the row
    let e = schedules::remove(&ctx.state, &now[0].id).await.unwrap_err();
    assert_eq!(
        e.downcast_ref::<TowerError>().unwrap().code,
        tower_core::ErrorCode::Conflict
    );
    workflows::set_enabled(&ctx.state, "digest", false, T0)
        .await
        .unwrap();
    assert!(!schedules::list(&ctx.state).await.unwrap()[0].enabled);
    workflows::set_enabled(&ctx.state, "digest", true, T0)
        .await
        .unwrap();
    assert!(schedules::list(&ctx.state).await.unwrap()[0].enabled);
}

#[tokio::test]
async fn an_event_trigger_starts_a_run_and_never_triggers_itself() {
    let ctx = boot_with(&["x"]).await;
    apply(
        &ctx,
        r#"
name = "on-block"
[inputs]
agent = { required = true }
[[triggers]]
kind = "event"
type = "agent.state"
match = { to = "blocked" }
input = { agent = "{{ event.subject_id }}" }
[[nodes]]
id = "a"
agent = "x"
prompt = "help {{ input.agent }}"
"#,
    )
    .await;
    let append = |to: &'static str| {
        let state = ctx.state.clone();
        async move {
            state
                .events
                .append(
                    tower_core::EventKind::AgentStateChange,
                    Some("agent"),
                    Some("ag_1"),
                    json!({ "from": "working", "to": to }),
                )
                .await
                .unwrap();
            workflows::drain(&state, T0).await.unwrap();
        }
    };
    append("idle").await;
    assert_eq!(count_runs(&ctx, "on-block").await, 0);
    append("blocked").await;
    assert_eq!(count_runs(&ctx, "on-block").await, 1);
    let run = workflows::list_runs(&ctx.state, &Default::default())
        .await
        .unwrap()[0]
        .clone();
    assert_eq!(run.input["agent"], "ag_1");
    assert_eq!(run.trigger["kind"], "event");
    append("blocked").await;
    assert_eq!(count_runs(&ctx, "on-block").await, 1, "skipped while open");

    // a trigger on task.completed: its own jobs don't count, others' do
    apply(
        &ctx,
        r#"
name = "follow"
[[triggers]]
kind = "event"
type = "task.completed"
[[nodes]]
id = "f"
agent = "x"
prompt = "follow up"
"#,
    )
    .await;
    complete(&ctx, &open_step(&ctx, &run, "a").await, json!("helped")).await;
    assert_eq!(
        count_runs(&ctx, "follow").await,
        1,
        "on-block's job completed"
    );
    let follow = workflows::list_runs(
        &ctx.state,
        &workflows::RunFilter {
            workflow: Some("follow".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap()[0]
        .clone();
    complete(
        &ctx,
        &open_step(&ctx, &follow, "f").await,
        json!("followed"),
    )
    .await;
    assert_eq!(
        count_runs(&ctx, "follow").await,
        1,
        "its own job never triggers it"
    );
}

fn tagged_job(title: &str, tags: &[&str], max_attempts: Option<i64>) -> tasks::CreateTask {
    tasks::CreateTask {
        title: title.into(),
        description: None,
        priority: None,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        assign: Some("w".into()),
        when_available: false,
        not_before: None,
        lease_s: None,
        max_attempts,
        pre_run: None,
        post_run: None,
    }
}

#[tokio::test]
async fn a_tagged_job_completing_or_failing_starts_the_workflow_that_names_its_tag() {
    let ctx = boot_with(&["x", "w"]).await;
    apply(
        &ctx,
        r#"
name = "release-notes"
[inputs]
change = { required = true }
[[triggers]]
kind = "event"
type = "task.completed"
match = { tags = "deploy" }
input = { change = "{{ event.title }}" }
[[nodes]]
id = "notes"
agent = "x"
prompt = "write notes for {{ input.change }}"
"#,
    )
    .await;
    apply(
        &ctx,
        r#"
name = "postmortem"
[[triggers]]
kind = "event"
type = "task.failed"
match = { tags = "deploy" }
[[nodes]]
id = "why"
agent = "x"
prompt = "find out why"
"#,
    )
    .await;
    let close = |t: tasks::CreateTask| {
        let state = ctx.state.clone();
        async move {
            let j = tasks::create(&state, t, T0).await.unwrap();
            tasks::report(
                &state,
                &j.id,
                "w",
                Some(TaskState::Completed),
                Some(json!("done")),
                T0,
            )
            .await
            .unwrap();
            workflows::drain(&state, T0).await.unwrap();
        }
    };

    close(tagged_job("ship v1.3", &["infra"], None)).await;
    assert_eq!(count_runs(&ctx, "release-notes").await, 0, "other tag");

    close(tagged_job("ship v1.4", &["infra", "deploy"], None)).await;
    assert_eq!(count_runs(&ctx, "release-notes").await, 1);
    let run = workflows::list_runs(&ctx.state, &Default::default())
        .await
        .unwrap()[0]
        .clone();
    assert_eq!(run.input["change"], "ship v1.4", "the job's title");
    assert_eq!(count_runs(&ctx, "postmortem").await, 0);

    // a lease that runs out on the last attempt fails the job: its tags
    // still reach the trigger
    let j = tasks::create(
        &ctx.state,
        tagged_job("ship v1.5", &["deploy"], Some(1)),
        T0,
    )
    .await
    .unwrap();
    let late = T0 + j.lease_s * 1000 + 1;
    assert!(tasks::expire_lease(&ctx.state, &j.id, late).await.unwrap());
    workflows::drain(&ctx.state, late).await.unwrap();
    assert_eq!(count_runs(&ctx, "postmortem").await, 1);
}

#[tokio::test]
async fn the_consumer_starts_at_the_head_so_old_events_fire_nothing() {
    let ctx = boot_with(&["x"]).await;
    ctx.state
        .events
        .append(
            tower_core::EventKind::AgentStateChange,
            Some("agent"),
            Some("ag_1"),
            json!({ "to": "blocked" }),
        )
        .await
        .unwrap();
    apply(
        &ctx,
        "name = \"late\"\n[[triggers]]\nkind = \"event\"\ntype = \"agent.state\"\n\
         [[nodes]]\nid = \"a\"\nagent = \"x\"\nprompt = \"p\"\n",
    )
    .await;
    workflows::drain(&ctx.state, T0).await.unwrap();
    assert_eq!(count_runs(&ctx, "late").await, 0);
}

// ---- artifacts -------------------------------------------------------------

#[tokio::test]
async fn artifacts_attach_to_the_owners_open_job_and_reach_the_next_step() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    apply(&ctx, ISSUE).await;
    let run = start(&ctx, "issue", json!({ "issue": "ENG-8" })).await;
    let triage = open_step(&ctx, &run, "triage").await;
    let task = triage.task_id.clone().unwrap();
    let path = format!("/v1/tasks/{task}/artifacts");

    let (status, v) = ctx
        .req(
            "POST",
            &path,
            Some(json!({ "as": "reviewer", "name": "n", "content": "c" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    let (status, v) = ctx
        .req(
            "POST",
            &path,
            Some(json!({ "as": "triager", "name": "n", "content": "x".repeat(1024 * 1024 + 1) })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let (status, v) = ctx
        .req(
            "POST",
            &path,
            Some(json!({ "as": "triager", "name": "notes.md", "content": "# long notes", "media_type": "text/markdown" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let id = v["artifact"]["id"].as_str().unwrap().to_string();

    let (_, shown) = ctx.req("GET", &format!("/v1/tasks/{task}"), None).await;
    assert_eq!(shown["artifacts"][0]["id"], id.as_str());
    let (_, got) = ctx.req("GET", &format!("/v1/artifacts/{id}"), None).await;
    assert_eq!(got["content"], "# long notes");

    complete(
        &ctx,
        &triage,
        json!({ "summary": "see notes", "outcome": "build" }),
    )
    .await;
    let d = job(&ctx, &open_step(&ctx, &run, "work").await)
        .await
        .description
        .unwrap();
    assert!(
        d.contains(&format!("artifact {id} `notes.md` (12 bytes)")),
        "{d}"
    );

    let (status, _) = ctx
        .req(
            "POST",
            &path,
            Some(json!({ "as": "triager", "name": "late", "content": "c" })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a closed job takes no artifacts"
    );
}

// ---- REST and MCP ----------------------------------------------------------

#[tokio::test]
async fn rest_apply_versions_validates_and_guards_removal() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    let (status, v) = ctx
        .req("POST", "/v1/workflows", Some(json!({ "source": ISSUE })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(
        (v["version"].as_i64(), v["changed"].as_bool()),
        (Some(1), Some(true))
    );
    let (status, v) = ctx
        .req("POST", "/v1/workflows", Some(json!({ "source": ISSUE })))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (v["version"].as_i64(), v["changed"].as_bool()),
        (Some(1), Some(false))
    );
    let changed = ISSUE.replace("Review {{", "Review closely {{");
    let (_, v) = ctx
        .req("POST", "/v1/workflows", Some(json!({ "source": changed })))
        .await;
    assert_eq!(v["version"], 2);

    let (status, v) = ctx
        .req(
            "POST",
            "/v1/workflows",
            Some(json!({ "source": "name = \"x\"\nnodes = 3" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/workflows",
            Some(json!({ "source": ISSUE.replace("\"reviewer\"", "\"ghost\"") })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("agent `ghost` not found"));

    let (_, list) = ctx.req("GET", "/v1/workflows", None).await;
    assert_eq!(list["workflows"][0]["current_version"], 2);
    assert_eq!(list["workflows"][0]["nodes"], 3);

    let (status, v) = ctx
        .req(
            "POST",
            "/v1/workflows/issue/runs",
            Some(json!({ "input": {} })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "missing input: {v}");
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/workflows/issue/runs",
            Some(json!({ "input": { "issue": "E-9" } })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let run_id = v["run"]["id"].as_str().unwrap().to_string();
    assert_eq!(v["run"]["version"], 2);

    let (status, _) = ctx.req("DELETE", "/v1/workflows/issue", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "a run is running");
    let (_, shown) = ctx.req("GET", &format!("/v1/runs/{run_id}"), None).await;
    assert_eq!(shown["steps"][0]["node_id"], "triage");
    let (status, _) = ctx
        .req("POST", &format!("/v1/runs/{run_id}/cancel"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = ctx.req("DELETE", "/v1/workflows/issue", None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, runs) = ctx.req("GET", "/v1/runs?workflow=issue", None).await;
    assert_eq!(
        runs["runs"][0]["state"], "canceled",
        "history survives removal"
    );

    let (status, _) = ctx.req("POST", "/v1/workflows/issue/runs", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_disabled_workflow_refuses_manual_starts() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    apply(&ctx, ISSUE).await;
    let (status, _) = ctx.req("POST", "/v1/workflows/issue/disable", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, v) = ctx
        .req(
            "POST",
            "/v1/workflows/issue/runs",
            Some(json!({ "input": { "issue": "E" } })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
}

#[tokio::test]
async fn agents_start_and_read_runs_over_mcp_but_only_the_operator_defines_or_cancels() {
    let ctx = boot_with(ISSUE_AGENTS).await;
    let r = ctx
        .tool(
            Some("triager"),
            "tower_workflow_apply",
            json!({ "source": ISSUE }),
        )
        .await;
    assert_eq!(r["isError"], true, "{r}");
    let r = ctx
        .tool(None, "tower_workflow_apply", json!({ "source": ISSUE }))
        .await;
    assert_ne!(r["isError"], true, "{r}");

    let r = ctx
        .tool(
            Some("triager"),
            "tower_workflow_run",
            json!({ "name": "issue", "input": { "issue": "E-1" } }),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    let run = r["structuredContent"]["run"].clone();
    assert_eq!(run["trigger"]["by"], "triager");
    let run_id = run["id"].as_str().unwrap();

    let r = ctx
        .tool(
            Some("triager"),
            "tower_run_show",
            json!({ "run_id": run_id }),
        )
        .await;
    let task = r["structuredContent"]["steps"][0]["task_id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = ctx
        .tool(
            Some("triager"),
            "tower_artifact_put",
            json!({ "task_id": task, "name": "n", "content": "body" }),
        )
        .await;
    assert_ne!(r["isError"], true, "{r}");
    let art = r["structuredContent"]["artifact"]["id"].as_str().unwrap();
    let r = ctx
        .tool(
            Some("builder"),
            "tower_artifact_get",
            json!({ "artifact_id": art }),
        )
        .await;
    assert_eq!(r["structuredContent"]["content"], "body");

    let r = ctx
        .tool(
            Some("triager"),
            "tower_run_cancel",
            json!({ "run_id": run_id }),
        )
        .await;
    assert_eq!(r["isError"], true);
    let r = ctx
        .tool(Some("triager"), "tower_workflow_list", json!({}))
        .await;
    assert_eq!(r["structuredContent"]["workflows"][0]["open_runs"], 1);
    let r = ctx
        .tool(None, "tower_run_cancel", json!({ "run_id": run_id }))
        .await;
    assert_eq!(r["structuredContent"]["run"]["state"], "canceled");
}
