//! Session tree traversal utilities.

use y_core::session::{SessionNode, SessionStore};
use y_core::types::SessionId;

use crate::error::SessionManagerError;

/// Utility functions for navigating the session tree.
pub struct TreeUtils;

impl TreeUtils {
    /// Find the root session node for a given session.
    pub async fn find_root(
        store: &dyn SessionStore,
        session_id: &SessionId,
    ) -> Result<SessionNode, SessionManagerError> {
        let node = store.get(session_id).await?;
        if node.root_id == node.id {
            Ok(node)
        } else {
            Ok(store.get(&node.root_id).await?)
        }
    }

    /// Collect the full path from root to a session (inclusive).
    ///
    /// Returns [root, ..., parent, self].
    pub async fn path_to_root(
        store: &dyn SessionStore,
        session_id: &SessionId,
    ) -> Result<Vec<SessionNode>, SessionManagerError> {
        let node = store.get(session_id).await?;
        let mut ancestors = store.ancestors(session_id).await?;
        ancestors.push(node);
        Ok(ancestors)
    }

    /// Find all leaf nodes (sessions with no children) in a subtree.
    pub async fn find_leaves(
        store: &dyn SessionStore,
        root_id: &SessionId,
    ) -> Result<Vec<SessionNode>, SessionManagerError> {
        let filter = y_core::session::SessionFilter {
            root_id: Some(root_id.clone()),
            ..Default::default()
        };
        let all = store.list(&filter).await?;

        let mut leaves = Vec::new();
        for node in &all {
            let children = store.children(&node.id).await?;
            if children.is_empty() {
                leaves.push(node.clone());
            }
        }

        Ok(leaves)
    }

    /// Count all nodes in a subtree rooted at the given node.
    pub async fn subtree_size(
        store: &dyn SessionStore,
        root_id: &SessionId,
    ) -> Result<usize, SessionManagerError> {
        let filter = y_core::session::SessionFilter {
            root_id: Some(root_id.clone()),
            ..Default::default()
        };
        let all = store.list(&filter).await?;
        Ok(all.len())
    }

    /// Get the depth of the deepest node in the subtree.
    pub async fn max_depth(
        store: &dyn SessionStore,
        root_id: &SessionId,
    ) -> Result<u32, SessionManagerError> {
        let filter = y_core::session::SessionFilter {
            root_id: Some(root_id.clone()),
            ..Default::default()
        };
        let all = store.list(&filter).await?;
        Ok(all.iter().map(|n| n.depth).max().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y_core::session::{CreateSessionOptions, SessionType};

    async fn setup_store() -> y_storage::SqliteSessionStore {
        let config = y_storage::StorageConfig::in_memory();
        let pool = y_storage::create_pool(&config).await.unwrap();
        y_storage::migration::run_embedded_migrations(&pool)
            .await
            .unwrap();
        y_storage::SqliteSessionStore::new(pool)
    }

    #[tokio::test]
    async fn test_find_root_from_child() {
        let store = setup_store().await;
        let root = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: Some("Root".into()),
            })
            .await
            .unwrap();

        let child = store
            .create(CreateSessionOptions {
                parent_id: Some(root.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let found_root = TreeUtils::find_root(&store, &child.id).await.unwrap();
        assert_eq!(found_root.id, root.id);
    }

    #[tokio::test]
    async fn test_find_root_from_root() {
        let store = setup_store().await;
        let root = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let found = TreeUtils::find_root(&store, &root.id).await.unwrap();
        assert_eq!(found.id, root.id);
    }

    #[tokio::test]
    async fn test_path_to_root() {
        let store = setup_store().await;
        let root = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let child = store
            .create(CreateSessionOptions {
                parent_id: Some(root.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let grandchild = store
            .create(CreateSessionOptions {
                parent_id: Some(child.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let path = TreeUtils::path_to_root(&store, &grandchild.id)
            .await
            .unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0].id, root.id);
        assert_eq!(path[1].id, child.id);
        assert_eq!(path[2].id, grandchild.id);
    }

    #[tokio::test]
    async fn test_find_leaves() {
        let store = setup_store().await;
        let root = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let _child1 = store
            .create(CreateSessionOptions {
                parent_id: Some(root.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let child2 = store
            .create(CreateSessionOptions {
                parent_id: Some(root.id.clone()),
                session_type: SessionType::Branch,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        // child2 has a grandchild — so child2 is NOT a leaf.
        let _grandchild = store
            .create(CreateSessionOptions {
                parent_id: Some(child2.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let leaves = TreeUtils::find_leaves(&store, &root.id).await.unwrap();
        // child1 and grandchild are leaves.
        assert_eq!(leaves.len(), 2);
    }

    #[tokio::test]
    async fn test_subtree_size() {
        let store = setup_store().await;
        let root = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let _c1 = store
            .create(CreateSessionOptions {
                parent_id: Some(root.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let _c2 = store
            .create(CreateSessionOptions {
                parent_id: Some(root.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let size = TreeUtils::subtree_size(&store, &root.id).await.unwrap();
        assert_eq!(size, 3); // root + 2 children
    }

    #[tokio::test]
    async fn test_max_depth() {
        let store = setup_store().await;
        let root = store
            .create(CreateSessionOptions {
                parent_id: None,
                session_type: SessionType::Main,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let child = store
            .create(CreateSessionOptions {
                parent_id: Some(root.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let _grandchild = store
            .create(CreateSessionOptions {
                parent_id: Some(child.id.clone()),
                session_type: SessionType::Child,
                agent_id: None,
                title: None,
            })
            .await
            .unwrap();

        let depth = TreeUtils::max_depth(&store, &root.id).await.unwrap();
        assert_eq!(depth, 2);
    }
}
