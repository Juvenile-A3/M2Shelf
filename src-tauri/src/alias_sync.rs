use std::sync::atomic::{AtomicBool, Ordering};

use crate::{
    bangumi,
    db::{AppResult, Database},
    models::BangumiSubject,
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
struct ActiveGuard;
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

/// One resumable, deduplicated pass. Completed Subjects (including empty aliases) never refetch.
pub fn sync_pending(database: &Database) -> AppResult<bool> {
    if ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(false);
    }
    let _guard = ActiveGuard;
    sync_with(database, bangumi::enrich_subject)
}

fn sync_with(
    database: &Database,
    mut fetch: impl FnMut(&BangumiSubject) -> AppResult<BangumiSubject>,
) -> AppResult<bool> {
    let mut changed = false;
    for subject in database.pending_alias_subjects()? {
        // Keep progress on network failure; retry only unfinished Subjects on a later launch.
        let detail = match fetch(&subject) {
            Ok(detail) => detail,
            Err(error) if bangumi::is_provider_wide_detail_error(&error) => break,
            Err(_) => continue,
        };
        if detail.subject_id != subject.subject_id || detail.subject_type != subject.subject_type {
            break;
        }
        changed |= database.complete_provider_alias_sync(&detail)?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Database) {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("test.db");
        let database = Database::new(path.clone());
        database.migrate().unwrap();
        let root_path = temp.path().join("media");
        std::fs::create_dir(&root_path).unwrap();
        let root = database.add_root(&root_path, None).unwrap();
        let connection = Connection::open(path).unwrap();
        for (id, subject) in [(1, 10), (2, 10), (3, 20), (4, 30)] {
            connection.execute("INSERT INTO nodes(id,library_root_id,absolute_path,folder_name,display_name,node_type) VALUES(?1,?2,?3,'Show','Show','WORK')", params![id,root.id,format!("fixture-{id}")]).unwrap();
            connection.execute("INSERT INTO metadata_bindings(node_id,provider,provider_subject_id,provider_subject_type,provider_title) VALUES(?1,'BANGUMI',?2,2,'Show')", params![id,subject]).unwrap();
        }
        (temp, database)
    }

    #[test]
    fn backfill_deduplicates_persists_empty_results_and_resumes_only_unfinished_subjects() {
        let (_temp, database) = fixture();
        let mut calls = Vec::new();
        assert!(sync_with(&database, |subject| {
            calls.push(subject.subject_id);
            if subject.subject_id == 30 {
                return Err("api.bgm.tv 返回 HTTP 503".into());
            }
            let mut detail = subject.clone();
            if subject.subject_id == 10 {
                detail.match_aliases = vec!["Official alias".into()];
            }
            Ok(detail)
        })
        .unwrap());
        assert_eq!(calls, vec![10, 20, 30]);
        assert_eq!(database.search("Official alias", None).unwrap().len(), 2);
        assert_eq!(
            database
                .pending_alias_subjects()
                .unwrap()
                .iter()
                .map(|s| s.subject_id)
                .collect::<Vec<_>>(),
            vec![30]
        );
        assert!(!sync_with(&database, |subject| Ok(subject.clone())).unwrap());
        assert!(!sync_with(&database, |_| panic!("completed Subjects must not refetch")).unwrap());
        // A later binding of the same Subject reuses completed aliases, even when its search row
        // omitted aliases (e.g. optional network enrichment failed).
        let subject = BangumiSubject {
            subject_id: 10,
            subject_type: 2,
            title: "Show".into(),
            title_cn: None,
            title_en: None,
            title_ja: None,
            title_ko: None,
            match_aliases: vec![],
            date: None,
            image_url: None,
            summary: None,
        };
        database.save_confirmed_binding(4, &subject).unwrap();
        assert_eq!(database.search("Official alias", None).unwrap().len(), 3);
        assert!(database.pending_alias_subjects().unwrap().is_empty());
    }

    #[test]
    fn stale_subject_does_not_block_others_and_provider_failure_preserves_pending_work() {
        let (_temp, database) = fixture();
        let mut calls = Vec::new();
        assert!(!sync_with(&database, |subject| {
            calls.push(subject.subject_id);
            if subject.subject_id == 10 {
                Err("api.bgm.tv 返回 HTTP 404".into())
            } else {
                Ok(subject.clone())
            }
        })
        .unwrap());
        assert_eq!(calls, vec![10, 20, 30]);
        assert_eq!(database.pending_alias_subjects().unwrap().len(), 1);
        assert!(!sync_with(&database, |_| Err("api.bgm.tv 返回 HTTP 503".into())).unwrap());
        assert_eq!(database.pending_alias_subjects().unwrap().len(), 1);
    }
}
