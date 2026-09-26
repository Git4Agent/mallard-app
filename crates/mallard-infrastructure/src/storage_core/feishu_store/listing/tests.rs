use super::*;
use crate::storage_core::feishu_store::mock::{MockFeishuServer as Server, ROOT};
use crate::storage_core::{library_object_key, LIBRARY_PREFIX, TAG_FILE};

fn repository(index: usize) -> String {
    format!("library-{index:032x}")
}

fn queries(count: usize) -> Vec<ObjectQuery> {
    (0..count)
        .flat_map(|index| {
            let id = repository(index);
            [
                ObjectQuery::Prefix(
                    ObjectPrefix::parse(format!("{LIBRARY_PREFIX}/{id}/_commits")).unwrap(),
                ),
                ObjectQuery::Key(library_object_key(&id, TAG_FILE).unwrap()),
            ]
        })
        .collect()
}

fn library(server: &Server) -> (String, String) {
    let mallard = server.add_folder(ROOT, ".mallard");
    let v1 = server.add_folder(&mallard, "v1");
    let library = server.add_folder(&v1, "library");
    (v1, library)
}

fn record(server: &Server, parent: &str, revision: usize) -> String {
    let commits = server.add_folder(parent, "_commits");
    let shard = server.add_folder(&commits, "0000000");
    server.add_file(
        &shard,
        &format!("{revision:010}.commit-test.root.0000000000001.json"),
        b"record",
    )
}

#[test]
fn absent_repositories_take_four_folder_reads_instead_of_thirty() {
    let server = Server::new();
    library(&server);
    let queries = queries(6);
    let start = server.count("files");
    let batch = server.store.list_objects_batch(&queries, 100);
    assert!(batch
        .iter()
        .all(|result| result.as_ref().unwrap().is_empty()));
    assert_eq!(server.count("files") - start, 4);
    assert!(server.store.cache.lock().unwrap().folders.is_empty());
    assert!(server.store.cache.lock().unwrap().files.is_empty());

    // The previous Library loop, on a separate store with no path state.
    let previous = server.second_store();
    let start = server.count("files");
    for query in &queries {
        match query {
            ObjectQuery::Prefix(prefix) => {
                assert!(previous.list_objects(prefix, 100).unwrap().is_empty())
            }
            ObjectQuery::Key(key) => assert!(previous.get_metadata(key, 1024).unwrap().is_none()),
        }
    }
    assert_eq!(server.count("files") - start, 30);
}

#[test]
fn next_batch_sees_new_repositories_and_same_named_ancestor_siblings() {
    let server = Server::new();
    let (v1, first_library) = library(&server);
    let queries = queries(6);
    assert!(server
        .store
        .list_objects_batch(&queries, 100)
        .iter()
        .all(|result| result.as_ref().unwrap().is_empty()));

    let first = server.add_folder(&first_library, &repository(0));
    server.add_file(&first, TAG_FILE, b"tag-only");
    let second_library = server.add_folder(&v1, "library");
    let second = server.add_folder(&second_library, &repository(1));
    server.add_file(&second, TAG_FILE, b"tag");
    record(&server, &second, 1);
    let ignored = server.add_folder(&second, "_objects");
    server.add_folder(&ignored, "00");
    let claims = server.add_folder(&second, "_claims");

    let start = server.count("files");
    let batch = server.store.list_objects_batch(&queries, 100);
    let sizes = batch
        .iter()
        .map(|result| result.as_ref().unwrap().len())
        .collect::<Vec<_>>();
    assert_eq!(sizes, [0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(server.count("files") - start, 9);
    let state = server.state.lock().unwrap();
    assert!(!state.listed_folders.contains(&ignored));
    assert!(!state.listed_folders.contains(&claims));
    drop(state);
    let reads = batch
        .into_iter()
        .flat_map(|result| result.unwrap())
        .collect::<Vec<_>>();
    let before = server.count("files");
    let bytes = server.store.get_listed_metadata_many(&reads, 1024).unwrap();
    assert!(bytes.iter().all(Option::is_some));
    assert_eq!(
        server.count("files"),
        before,
        "listed tokens need no path lookup"
    );
    assert!(server.store.cache.lock().unwrap().folders.is_empty());
    assert!(server.store.cache.lock().unwrap().files.is_empty());
}

#[test]
fn batch_consumes_pages_and_unions_same_named_repositories() {
    let server = Server::new();
    let (_, library) = library(&server);
    let first = server.add_folder(&library, &repository(0));
    let second = server.add_folder(&library, &repository(0));
    let first_tag = server.add_file(&first, TAG_FILE, b"same-tag");
    server.add_file(&second, TAG_FILE, b"same-tag");
    record(&server, &first, 1);
    record(&server, &second, 2);
    let unrelated = server.add_folder(&library, &repository(99));
    let payloads = server.add_folder(&first, "_objects");
    server.add_folder(&payloads, "aa");
    server.state.lock().unwrap().page_size = 1;

    let batch = server.store.list_objects_batch(&queries(1), 100);
    assert_eq!(batch[0].as_ref().unwrap().len(), 2);
    let tags = batch[1].as_ref().unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].feishu.as_ref().unwrap().token, first_tag);
    let state = server.state.lock().unwrap();
    assert_eq!(
        state
            .listed_folders
            .iter()
            .filter(|token| **token == library)
            .count(),
        3
    );
    assert_eq!(
        state
            .listed_folders
            .iter()
            .filter(|token| **token == first)
            .count(),
        3
    );
    assert_eq!(
        state
            .listed_folders
            .iter()
            .filter(|token| **token == second)
            .count(),
        2
    );
    assert!(!state.listed_folders.contains(&unrelated));
    assert!(!state.listed_folders.contains(&payloads));
}

#[test]
fn incomplete_prefix_is_an_error_without_hiding_other_results() {
    let server = Server::new();
    let (_, library) = library(&server);
    let repo = server.add_folder(&library, &repository(0));
    server.add_file(&repo, TAG_FILE, b"tag");
    record(&server, &repo, 1);
    record(&server, &repo, 2);
    let batch = server.store.list_objects_batch(&queries(2), 1);
    assert!(batch[0].as_ref().unwrap_err().contains("safety limit"));
    assert_eq!(batch[1].as_ref().unwrap().len(), 1);
    assert!(batch[2].as_ref().unwrap().is_empty());
    assert!(batch[3].as_ref().unwrap().is_empty());
}
