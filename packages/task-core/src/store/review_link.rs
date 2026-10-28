//! Explicit implementation review reconciliation (never infer from ordinary links).
use super::*;

pub(super) fn is_implementation_review(task: &Task) -> bool {
    task.category.as_deref() == Some("Implementation Review") ||
    task.question.as_ref().and_then(|q| q.request_type.as_deref()) == Some("visual-review")
}

pub(super) fn close_question(task: &mut Task, was_for_user: bool) {
    if !was_for_user || task.completed.is_none() ||
       !is_implementation_review(task) { return; }
    if let Some(ref mut q) = task.question {
        if q.answer.as_deref().is_none_or(|a| a.trim().is_empty()) {
            q.answer = Some(if task.user_notes.trim().is_empty() {
                "Approved".into()
            } else { "Disposition unclear".into() });
        }
    }
    task.attention.retain(|a| *a != TaskAttention::Question);
}

pub(super) fn reconcile(_root: &Path, snapshot: &BoardSnapshot, review: &Task) -> Result<(), TaskStoreError> {
    if review.completed.is_none() || !is_implementation_review(review) ||
       review.related_to.len() != 1 { return Ok(()); }
    let target = &review.related_to[0];
    if target.contains('#') || target == &review.id { return Ok(()); }
    let Some(existing) = snapshot.tasks.iter().find(|t| t.id == *target && t.kind == TaskKind::Task) else {
        return Ok(());
    };
    let path = PathBuf::from(&existing.path);
    let original = fs::read_to_string(&path)?;
    let mut parent = parse_task(&path, existing.archived, &original)?;
    let marker = format!("[review:{}]", review.id);
    if parent.notes.contains(&marker) { return Ok(()); }
    let answer = review.question.as_ref().and_then(|q| q.answer.as_deref()).unwrap_or("").to_lowercase();
    let negative = answer.trim() == "needs changes" || answer.trim() == "rejected";
    let positive = matches!(answer.trim(), "approved" | "approve" | "accepted");
    let scope_valid = !review.acceptance_criteria.is_empty() &&
        review.acceptance_criteria.iter().all(|item| parent.acceptance_criteria.contains(item));
    let full = scope_valid && review.acceptance_criteria.len() == parent.acceptance_criteria.len();
    let blocked = is_task_blocked(&parent);
    let disposition = if negative {
        if !blocked {
            parent.column = if parent.claimed_by.is_some() { IN_PROGRESS_COLUMN_ID.into() } else { AI_READY_COLUMN_ID.into() };
            apply_completion_state(&snapshot.config, &mut parent);
        }
        "needs_changes"
    } else if positive || (answer.trim().is_empty() && review.user_notes.is_empty()) {
        if full && !blocked && ensure_dependencies_complete(snapshot, &parent).is_ok() &&
            !parent.attention.contains(&TaskAttention::NeedsUser) {
            if let Some(done) = snapshot.config.columns.iter().find(|c| c.terminal) {
                parent.column = done.id.clone();
                apply_completion_state(&snapshot.config, &mut parent);
            }
            "approved_parent_done"
        } else { "partial_or_blocked" }
    } else { "ambiguous_feedback" };
    if !review.user_notes.is_empty() {
        if !parent.user_notes.is_empty() { parent.user_notes.push_str("\n\n"); }
        parent.user_notes.push_str(&format!("Review {}:\n{}", review.id, review.user_notes));
    }
    let remaining: Vec<&str> = parent.acceptance_criteria.iter()
        .filter(|criterion| !review.acceptance_criteria.contains(criterion))
        .map(String::as_str).collect();
    parent.notes.push_str(&format!("\n{} {} at {}. Accepted scope: [{}]. Remaining: [{}].", marker, disposition,
        review.completed_at.as_deref().unwrap_or("unknown"),
        review.acceptance_criteria.join("; "), remaining.join("; ")));
    validate_task(snapshot, &parent)?;
    write_text_atomic(&path, &render_task(&parent, Some(&original))?)?;
    Ok(())
}
