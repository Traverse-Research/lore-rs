use crate::LoreStringExt;
use lore_sys::{
    lore_address_t, lore_branch_id_t, lore_branch_location_t, lore_branch_point_t, lore_context_t,
    lore_error_code_t, lore_event_t, lore_event_tag_t, lore_file_action_t,
    lore_file_stage_count_data_t, lore_file_unstage_count_data_t, lore_fragment_t, lore_hash_t,
    lore_log_level_t, lore_node_id_t, lore_node_type_t, lore_repository_clone_count_data_t,
    lore_repository_id_t, lore_revision_commit_count_data_t, LORE_EVENT_AUTH_IDENTITY,
    LORE_EVENT_AUTH_URL, LORE_EVENT_AUTH_USER_INFO, LORE_EVENT_AUTH_USER_TOKEN,
    LORE_EVENT_BRANCH_ARCHIVE, LORE_EVENT_BRANCH_CREATE, LORE_EVENT_BRANCH_INFO,
    LORE_EVENT_BRANCH_LIST_ENTRY, LORE_EVENT_BRANCH_PUSH, LORE_EVENT_BRANCH_PUSH_FRAGMENT_END,
    LORE_EVENT_BRANCH_SWITCH_END, LORE_EVENT_COMPLETE, LORE_EVENT_END, LORE_EVENT_ERROR,
    LORE_EVENT_FILE_INFO, LORE_EVENT_FILE_STAGE_END, LORE_EVENT_FILE_STAGE_FILE,
    LORE_EVENT_FILE_STAGE_REVISION, LORE_EVENT_FILE_UNSTAGE_END, LORE_EVENT_FILE_UNSTAGE_FILE,
    LORE_EVENT_FILE_UNSTAGE_REVISION, LORE_EVENT_LOG, LORE_EVENT_REPOSITORY_CLONE_END,
    LORE_EVENT_REPOSITORY_DATA, LORE_EVENT_REPOSITORY_STATE_DUMP_NODE,
    LORE_EVENT_REPOSITORY_STATUS_COUNT, LORE_EVENT_REPOSITORY_STATUS_FILE,
    LORE_EVENT_REPOSITORY_STATUS_REVISION, LORE_EVENT_REPOSITORY_STATUS_SUMMARY,
    LORE_EVENT_REVISION_COMMIT_END, LORE_EVENT_REVISION_COMMIT_REVISION,
    LORE_EVENT_REVISION_HISTORY, LORE_EVENT_REVISION_HISTORY_ENTRY, LORE_EVENT_REVISION_INFO,
    LORE_EVENT_REVISION_SYNC_REVISION, LORE_EVENT_REVISION_SYNC_TARGET,
    LORE_EVENT_REVISION_TREE_CHILD, LORE_EVENT_REVISION_TREE_CLOSE_COMPLETE,
    LORE_EVENT_REVISION_TREE_INFO, LORE_EVENT_REVISION_TREE_LIST_CHILDREN_BEGIN,
    LORE_EVENT_REVISION_TREE_LOADED, LORE_EVENT_REVISION_TREE_NODE_INFO,
    LORE_EVENT_REVISION_TREE_NODE_PATH, LORE_EVENT_REVISION_TREE_RESOLVE_PATH_COMPLETE,
    LORE_EVENT_STORAGE_GET_DATA, LORE_EVENT_STORAGE_GET_HEADER,
    LORE_EVENT_STORAGE_GET_ITEM_COMPLETE, LORE_EVENT_STORAGE_GET_METADATA_ITEM_COMPLETE,
    LORE_EVENT_STORAGE_OPENED, LORE_EVENT_STORAGE_PUT_ITEM_COMPLETE, LORE_LOG_LEVEL_ERROR,
    LORE_LOG_LEVEL_INFO, LORE_LOG_LEVEL_TRACE, LORE_LOG_LEVEL_WARN,
};

/// One event decoded from lore's tagged event union. Borrows from the raw
/// event. Not all variants are implemented yet; more will be added as
/// required.
///
/// Variants never store raw Lore types that carry pointers internally, such
/// as `lore_string_t`, to enforce correct lifetimes. Instead those fields are
/// decoded to references (`&str`, `&[u8]`) bound to `'a`.
///
/// Every call ends with [`Self::Complete`] followed by [`Self::End`]. A
/// terminal failure is reported on the completion alone: its `status` and
/// `error_code` are the same value, and no [`Self::Error`] precedes it. An
/// [`Self::Error`] mid-stream is a non-fatal one.
#[non_exhaustive]
pub enum Event<'a> {
    Log {
        level: lore_log_level_t,
        message: &'a str,
    },
    /// A non-fatal error. Its `error_type` is a legacy code that does not
    /// match the completion's `error_code` for most errors.
    Error {
        error_type: u32,
        message: &'a str,
    },
    Complete {
        /// Zero on success, otherwise the same code as `error_code`.
        status: i32,
        /// The error's FFI code; zero on success, `-1` for an internal error.
        /// Lore does not yet promise these values stable.
        error_code: i32,
        /// The failure's message, empty when the call succeeded.
        message: &'a str,
    },
    /// The last event of a call, after [`Self::Complete`]. Nothing arrives
    /// after it.
    End,
    /// Where to go to log in, from an interactive login asked not to open a
    /// browser. No function in [`call`](crate::call) emits this yet.
    AuthUrl {
        url: &'a str,
    },
    /// Who Lore resolved an identity to. Concludes a successful login.
    AuthUserInfo {
        id: &'a str,
        /// The token's `preferred_username`, its `name`, or `id`, whichever
        /// Lore has first, so never empty.
        name: &'a str,
    },
    /// An identity together with its token. No function in
    /// [`call`](crate::call) emits this yet.
    AuthUserToken {
        id: &'a str,
        name: &'a str,
        /// A credential: a callback that logs its events unfiltered logs this.
        token: &'a str,
        preferred_username: &'a str,
        service_account: bool,
        /// **Milliseconds** since the Unix epoch — unlike the `created` fields
        /// elsewhere, which are seconds. Zero when Lore cannot say.
        expires: u64,
    },
    /// One entry of the token store.
    AuthIdentity {
        auth_url: &'a str,
        /// Empty for an authentication token, a repository id for an
        /// authorization one.
        resource: &'a str,
        user_id: &'a str,
        /// Acceptable root domains as one `", "`-joined string. Empty means
        /// unrestricted rather than none.
        authorized_domains: &'a str,
        /// **Milliseconds** since the Unix epoch — unlike the `created` fields
        /// elsewhere, which are seconds. Zero when Lore cannot say.
        expires: u64,
        /// Empty unless the call asked for tokens.
        token: &'a str,
    },
    /// The `stack` of branch points the raw event also carries is not decoded:
    /// it is an array, which no variant here has needed yet, and
    /// [`Self::BranchInfo::branch_point`] is its first entry.
    BranchInfo {
        id: lore_branch_id_t,
        name: &'a str,
        category: &'a str,
        /// Tip in the local mutable store, zero when there is none.
        latest: lore_hash_t,
        /// Tip on the remote, zero when there is no remote to ask or nothing
        /// has been pushed.
        latest_remote: lore_hash_t,
        parent: lore_branch_id_t,
        /// Revision on `parent` this branch was created from.
        branch_point: lore_hash_t,
        creator: &'a str,
        created: u64,
        archived: bool,
    },
    /// Concludes a successful `branch_archive`.
    BranchArchive {
        name: &'a str,
    },
    /// A branch was created. Lore emits it only when the branch points at a
    /// revision; a `branch_create` on an empty branch succeeds without it.
    BranchCreate {
        name: &'a str,
        /// The revision the new branch points at.
        latest: lore_hash_t,
        /// Lore made a new revision for the branch, which happens when the
        /// repository has linked repositories that needed their own branch.
        is_commit: bool,
    },
    BranchListEntry {
        /// Which side's view of the branch this is. A branch known both
        /// locally and on the remote is listed once for each.
        location: lore_branch_location_t,
        id: lore_branch_id_t,
        name: &'a str,
        category: &'a str,
        /// Tip on the side `location` names, zero when there is none.
        latest: lore_hash_t,
        /// The branch points this branch was created from.
        stack: &'a [lore_branch_point_t],
        creator: &'a str,
        /// Milliseconds since the Unix epoch for a branch created by a client,
        /// seconds for one created on a server.
        created: u64,
        /// Whether the instance is on this branch.
        is_current: bool,
        archived: bool,
    },
    /// One per repository a `branch_push` pushes, with the tips on both sides
    /// before any of its data moves.
    BranchPush {
        remote: &'a str,
        repository: lore_repository_id_t,
        branch: lore_branch_id_t,
        branch_name: &'a str,
        /// Zero when the remote does not have the branch yet.
        remote_revision: lore_hash_t,
        local_revision: lore_hash_t,
        /// Revisions on the remote since the two sides last agreed.
        remote_history: u64,
        /// Revisions the push will send.
        local_history: u64,
        /// The remote already holds everything local has; nothing is sent.
        already_pushed: bool,
        /// The branch is the repository's default branch.
        is_default: bool,
        /// The push is of a linked repository rather than the main one.
        is_link: bool,
        /// The push is of a layer rather than the main repository.
        is_layer: bool,
    },
    /// The upload of one revision. It does not say which repository the
    /// revision is in.
    BranchPushFragmentEnd {
        fragments: u64,
        bytes_transferred: u64,
    },
    /// Concludes a successful `branch_switch` with the branch it landed on;
    /// the fields of Lore's nested `lore_branch_switch_data_t`.
    BranchSwitchEnd {
        id: lore_branch_id_t,
        name: &'a str,
        /// Tip in the local store, zero when the instance never had the branch.
        latest_local: lore_hash_t,
        /// Tip on the remote, zero when there is none.
        latest_remote: lore_hash_t,
        /// The revision the working tree is now on.
        revision: lore_hash_t,
        /// Where the branch was found.
        location: lore_branch_location_t,
    },
    FileInfo {
        path: &'a str,
        hash: lore_hash_t,
        context: lore_context_t,
        is_file: bool,
        size: u64,
    },
    FileStageFile {
        path: &'a str,
        action: lore_file_action_t,
        /// Where a move or copy came from; empty for the other actions.
        from_path: &'a str,
    },
    /// Lore's tally of a `file_stage`, once it is done.
    FileStageEnd {
        count: lore_file_stage_count_data_t,
    },
    /// The staged state a `file_stage` produced, one per repository it changed.
    FileStageRevision {
        repository: lore_repository_id_t,
        revision: lore_hash_t,
    },
    /// Lore's tally of a `file_unstage`, once it is done.
    FileUnstageEnd {
        count: lore_file_unstage_count_data_t,
    },
    /// The staged state a `file_unstage` left behind.
    FileUnstageRevision {
        repository: lore_repository_id_t,
        revision: lore_hash_t,
    },
    FileUnstageFile {
        path: &'a str,
        action: lore_file_action_t,
    },
    RepositoryData {
        remote_url: &'a str,
        id: lore_repository_id_t,
        name: &'a str,
        description: &'a str,
        default_branch: lore_branch_id_t,
        default_branch_name: &'a str,
        creator: &'a str,
        created: u64,
    },
    /// Concludes a successful `repository_clone` with what was checked out.
    RepositoryCloneEnd {
        branch: &'a str,
        revision: lore_hash_t,
        count: lore_repository_clone_count_data_t,
    },
    RepositoryStatusRevision {
        repository: lore_repository_id_t,
        branch: lore_branch_id_t,
        branch_name: &'a str,
        /// The revision the repository is on.
        revision: lore_hash_t,
        revision_number: u64,
        /// Zero when nothing is staged.
        revision_staged: lore_hash_t,
        /// The incoming revision of a pending merge, zero when there is none.
        revision_merged: lore_hash_t,
        /// The last revision merged in from the parent branch; Lore only
        /// works it out when the sync-point option is set.
        revision_merged_parent_branch: lore_hash_t,
        /// Tip of the branch in the local store.
        revision_local: lore_hash_t,
        revision_local_number: u64,
        /// Tip of the branch on the remote; zero when unknown, when the
        /// branch is not on the remote, or when the remote is unavailable.
        revision_remote: lore_hash_t,
        /// Zero when `revision_remote` is.
        revision_remote_number: u64,
        /// The local store holds revisions that are not on the remote's
        /// history line.
        is_local_ahead: bool,
        /// The remote holds revisions that are not in the local store.
        is_remote_ahead: bool,
        /// A remote is configured and reachable with a local identity. This
        /// is connectivity only, not authorization.
        remote_available: bool,
        /// The remote answered the revision query authoritatively, so the
        /// identity may access the repository.
        remote_authorized: bool,
        /// The branch exists on the remote.
        remote_branch_exist: bool,
    },
    RepositoryStatusFile {
        path: &'a str,
        size: u64,
        action: lore_file_action_t,
        /// Lore's `type`.
        kind: lore_node_type_t,
        staged: bool,
        /// The change comes from a merge.
        merged: bool,
        conflict: bool,
        conflict_unresolved: bool,
        /// The conflict was resolved automatically.
        conflict_automerged: bool,
        /// The conflict was resolved with the local side.
        conflict_mine: bool,
        /// The conflict was resolved with the incoming side.
        conflict_theirs: bool,
        /// The file differs from the recorded state.
        dirty: bool,
        /// Where a move or copy came from; empty for the other actions.
        from_path: &'a str,
    },
    /// The size of the tree `repository_status` reported on, when `count` is
    /// set: the staged state when there is one, the current revision
    /// otherwise, filtered by the view.
    RepositoryStatusCount {
        directories: u64,
        files: u64,
    },
    /// The changes a `repository_status` with `scan` or `check_dirty` found,
    /// once it is done.
    RepositoryStatusSummary {
        adds: u64,
        deletes: u64,
        modifies: u64,
        moves: u64,
        copies: u64,
    },
    RepositoryStateDumpNode {
        path: &'a str,
    },
    /// The final tally of a `revision_commit`'s file processing for one
    /// repository, before that repository's revision is reported.
    RevisionCommitEnd {
        count: lore_revision_commit_count_data_t,
    },
    RevisionCommitRevision {
        repository: lore_repository_id_t,
        branch: lore_branch_id_t,
        revision: lore_hash_t,
        revision_number: u64,
        /// The direct parent, then the other parent of a merge; zero where
        /// there is none.
        parents: [lore_hash_t; 2],
    },
    /// Opens a `revision_history` with what it lists, along with its first
    /// entry; an empty history has none.
    RevisionHistory {
        repository: lore_repository_id_t,
        branch: lore_branch_id_t,
    },
    /// One revision of a `revision_history`, newest first.
    RevisionHistoryEntry {
        revision: lore_hash_t,
        revision_number: u64,
        /// The direct parent, then the other parent of a merge; zero where
        /// there is none.
        parents: [lore_hash_t; 2],
    },
    RevisionInfo {
        repository: lore_repository_id_t,
        /// Zero when the signature resolved to no revision.
        revision: lore_hash_t,
        revision_number: u64,
        /// The direct parent, then the other parent of a merge; zero where
        /// there is none.
        parents: [lore_hash_t; 2],
    },
    /// The revision a `revision_sync` or `branch_switch` left the working
    /// tree on. Emitted once; when the sync had to merge divergent history
    /// it is the merge revision, with `merge` set.
    RevisionSyncRevision {
        branch: lore_branch_id_t,
        revision: lore_hash_t,
        revision_number: u64,
        /// The revision is a merge Lore made to reconcile divergent history.
        merge: bool,
        /// The merge left conflicts for the caller to resolve.
        conflict: bool,
    },
    RevisionSyncTarget {
        remote: &'a str,
        repository: lore_repository_id_t,
        branch: lore_branch_id_t,
        branch_name: &'a str,
        source_revision: lore_hash_t,
        source_revision_number: u64,
        target_revision: lore_hash_t,
        target_revision_number: u64,
        /// The target is the branch tip.
        is_latest: bool,
        /// The target came from the local revision history, not the remote.
        local: bool,
    },
    RevisionTreeLoaded {
        handle_id: u64,
    },
    RevisionTreeResolvePathComplete {
        id: u64,
        node_id: lore_node_id_t,
        /// The tree the node belongs to, which differs from the handle's when
        /// the path crossed a sub-repository link.
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        error_code: lore_error_code_t,
    },
    /// Emitted once before the children of a listing, carrying the outcome
    /// and the tree the children belong to.
    RevisionTreeListChildrenBegin {
        id: u64,
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        error_code: lore_error_code_t,
    },
    RevisionTreeChild {
        id: u64,
        node_id: lore_node_id_t,
        name: &'a str,
        parent_id: lore_node_id_t,
        kind: u32,
        mode: u16,
        size: u64,
        address: lore_address_t,
        error_code: lore_error_code_t,
    },
    RevisionTreeNodeInfo {
        id: u64,
        node_id: lore_node_id_t,
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        name: &'a str,
        parent_id: lore_node_id_t,
        kind: u32,
        mode: u16,
        size: u64,
        address: lore_address_t,
        file_id: lore_context_t,
        error_code: lore_error_code_t,
    },
    RevisionTreeNodePath {
        id: u64,
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        /// The path from the root to the node.
        path: &'a str,
        error_code: lore_error_code_t,
    },
    RevisionTreeInfo {
        id: u64,
        repository: lore_repository_id_t,
        revision: lore_hash_t,
        /// The parent revisions, zero where there is none.
        parents: [lore_hash_t; 2],
        creation_timestamp: i64,
        author_identity: &'a str,
        metadata_key_count: u32,
        error_code: lore_error_code_t,
    },
    RevisionTreeCloseComplete {
        id: u64,
        error_code: lore_error_code_t,
    },
    StorageOpened {
        handle_id: u64,
    },
    /// The terminal event of a put item, success or failure. Also concludes
    /// a `put_file` item.
    StoragePutItemComplete {
        id: u64,
        /// The item's address on success, zero on failure.
        address: lore_address_t,
        error_code: lore_error_code_t,
    },
    StorageGetHeader {
        id: u64,
        address: lore_address_t,
        /// Size of the item's reassembled content, before any data arrives.
        size_content: u64,
    },
    StorageGetData {
        /// Which item of the call these bytes belong to.
        id: u64,
        offset: u64,
        bytes: &'a [u8],
    },
    StorageGetItemComplete {
        id: u64,
        /// The item's address on success, zero on failure.
        address: lore_address_t,
        error_code: lore_error_code_t,
    },
    /// The terminal event of a metadata lookup, success or failure. Carries
    /// no bytes, and no other event precedes it.
    StorageGetMetadataItemComplete {
        id: u64,
        /// The item's address on success, zero on failure.
        address: lore_address_t,
        /// What the store holds for the address; zeroed on failure.
        fragment: lore_fragment_t,
        error_code: lore_error_code_t,
    },
    /// Event kinds without a mapped variant (yet), add them here as needed
    Other {
        tag: lore_event_tag_t,
    },
}

impl<'a> Event<'a> {
    /// # Safety
    ///
    /// `event` must be a valid Lore event whose tag matches the union variant
    /// that is actually initialized, with any contained pointers valid for
    /// `lifetime 'a`.
    pub unsafe fn from_raw(event: &'a lore_event_t) -> Result<Self, std::str::Utf8Error> {
        let tag = event.tag;

        unsafe {
            let data = &event.__bindgen_anon_1;
            Ok(match tag {
                LORE_EVENT_LOG => Self::Log {
                    level: data.log.level,
                    message: data.log.message.try_to_str()?,
                },
                LORE_EVENT_ERROR => Self::Error {
                    error_type: data.error.error_type,
                    message: data.error.error_inner.try_to_str()?,
                },
                LORE_EVENT_COMPLETE => Self::Complete {
                    status: data.complete.status,
                    error_code: data.complete.error.error_code,
                    message: data.complete.error.message.try_to_str()?,
                },
                LORE_EVENT_END => Self::End,
                LORE_EVENT_AUTH_URL => Self::AuthUrl {
                    url: data.auth_url.url.try_to_str()?,
                },
                LORE_EVENT_AUTH_USER_INFO => Self::AuthUserInfo {
                    id: data.auth_user_info.id.try_to_str()?,
                    name: data.auth_user_info.name.try_to_str()?,
                },
                LORE_EVENT_AUTH_USER_TOKEN => Self::AuthUserToken {
                    id: data.auth_user_token.id.try_to_str()?,
                    name: data.auth_user_token.name.try_to_str()?,
                    token: data.auth_user_token.token.try_to_str()?,
                    preferred_username: data.auth_user_token.preferred_username.try_to_str()?,
                    service_account: data.auth_user_token.flag_service_account != 0,
                    expires: data.auth_user_token.expires,
                },
                LORE_EVENT_AUTH_IDENTITY => Self::AuthIdentity {
                    auth_url: data.auth_identity.auth_url.try_to_str()?,
                    resource: data.auth_identity.resource.try_to_str()?,
                    user_id: data.auth_identity.user_id.try_to_str()?,
                    authorized_domains: data.auth_identity.authorized_domains.try_to_str()?,
                    expires: data.auth_identity.expires,
                    token: data.auth_identity.token.try_to_str()?,
                },
                LORE_EVENT_BRANCH_INFO => Self::BranchInfo {
                    id: data.branch_info.id,
                    name: data.branch_info.name.try_to_str()?,
                    category: data.branch_info.category.try_to_str()?,
                    latest: data.branch_info.latest,
                    latest_remote: data.branch_info.latest_remote,
                    parent: data.branch_info.parent,
                    branch_point: data.branch_info.branch_point,
                    creator: data.branch_info.creator.try_to_str()?,
                    created: data.branch_info.created,
                    archived: data.branch_info.archived != 0,
                },
                LORE_EVENT_BRANCH_ARCHIVE => Self::BranchArchive {
                    name: data.branch_archive.name.try_to_str()?,
                },
                LORE_EVENT_BRANCH_CREATE => Self::BranchCreate {
                    name: data.branch_create.name.try_to_str()?,
                    latest: data.branch_create.latest,
                    is_commit: data.branch_create.is_commit != 0,
                },
                LORE_EVENT_BRANCH_LIST_ENTRY => Self::BranchListEntry {
                    location: data.branch_list_entry.location,
                    id: data.branch_list_entry.id,
                    name: data.branch_list_entry.name.try_to_str()?,
                    category: data.branch_list_entry.category.try_to_str()?,
                    latest: data.branch_list_entry.latest,
                    stack: {
                        // Lore hands out a null pointer for an empty array,
                        // which `from_raw_parts` does not accept even at
                        // length zero.
                        let stack = &data.branch_list_entry.stack;
                        if stack.ptr.is_null() || stack.count == 0 {
                            &[]
                        } else {
                            std::slice::from_raw_parts(stack.ptr, stack.count)
                        }
                    },
                    creator: data.branch_list_entry.creator.try_to_str()?,
                    created: data.branch_list_entry.created,
                    is_current: data.branch_list_entry.is_current != 0,
                    archived: data.branch_list_entry.archived != 0,
                },
                LORE_EVENT_BRANCH_PUSH => Self::BranchPush {
                    remote: data.branch_push.remote.try_to_str()?,
                    repository: data.branch_push.repository,
                    branch: data.branch_push.branch,
                    branch_name: data.branch_push.branch_name.try_to_str()?,
                    remote_revision: data.branch_push.remote_revision,
                    local_revision: data.branch_push.local_revision,
                    remote_history: data.branch_push.remote_history,
                    local_history: data.branch_push.local_history,
                    already_pushed: data.branch_push.flag_already_pushed != 0,
                    is_default: data.branch_push.flag_default != 0,
                    is_link: data.branch_push.flag_link != 0,
                    is_layer: data.branch_push.flag_layer != 0,
                },
                LORE_EVENT_BRANCH_PUSH_FRAGMENT_END => Self::BranchPushFragmentEnd {
                    fragments: data.branch_push_fragment_end.fragments,
                    bytes_transferred: data.branch_push_fragment_end.bytes_transferred,
                },
                LORE_EVENT_BRANCH_SWITCH_END => Self::BranchSwitchEnd {
                    id: data.branch_switch_end.branch.id,
                    name: data.branch_switch_end.branch.name.try_to_str()?,
                    latest_local: data.branch_switch_end.branch.latest_local,
                    latest_remote: data.branch_switch_end.branch.latest_remote,
                    revision: data.branch_switch_end.branch.revision,
                    location: data.branch_switch_end.branch.location,
                },
                LORE_EVENT_FILE_INFO => Self::FileInfo {
                    path: data.file_info.path.try_to_str()?,
                    hash: data.file_info.hash,
                    context: data.file_info.context,
                    is_file: data.file_info.is_file != 0,
                    size: data.file_info.size,
                },
                LORE_EVENT_FILE_STAGE_FILE => Self::FileStageFile {
                    path: data.file_stage_file.path.try_to_str()?,
                    action: data.file_stage_file.action,
                    from_path: data.file_stage_file.from_path.try_to_str()?,
                },
                LORE_EVENT_FILE_STAGE_END => Self::FileStageEnd {
                    count: data.file_stage_end.count,
                },
                LORE_EVENT_FILE_STAGE_REVISION => Self::FileStageRevision {
                    repository: data.file_stage_revision.repository,
                    revision: data.file_stage_revision.revision,
                },
                LORE_EVENT_FILE_UNSTAGE_END => Self::FileUnstageEnd {
                    count: data.file_unstage_end.count,
                },
                LORE_EVENT_FILE_UNSTAGE_REVISION => Self::FileUnstageRevision {
                    repository: data.file_unstage_revision.repository,
                    revision: data.file_unstage_revision.revision,
                },
                LORE_EVENT_FILE_UNSTAGE_FILE => Self::FileUnstageFile {
                    path: data.file_unstage_file.path.try_to_str()?,
                    action: data.file_unstage_file.action,
                },
                LORE_EVENT_REPOSITORY_DATA => Self::RepositoryData {
                    remote_url: data.repository_data.remote_url.try_to_str()?,
                    id: data.repository_data.id,
                    name: data.repository_data.name.try_to_str()?,
                    description: data.repository_data.description.try_to_str()?,
                    default_branch: data.repository_data.default_branch,
                    default_branch_name: data.repository_data.default_branch_name.try_to_str()?,
                    creator: data.repository_data.creator.try_to_str()?,
                    created: data.repository_data.created,
                },
                LORE_EVENT_REPOSITORY_CLONE_END => Self::RepositoryCloneEnd {
                    branch: data.repository_clone_end.branch.try_to_str()?,
                    revision: data.repository_clone_end.revision,
                    count: data.repository_clone_end.count,
                },
                LORE_EVENT_REPOSITORY_STATUS_REVISION => {
                    let status = &data.repository_status_revision;
                    Self::RepositoryStatusRevision {
                        repository: status.repository,
                        branch: status.branch,
                        branch_name: status.branch_name.try_to_str()?,
                        revision: status.revision,
                        revision_number: status.revision_number,
                        revision_staged: status.revision_staged,
                        revision_merged: status.revision_merged,
                        revision_merged_parent_branch: status.revision_merged_parent_branch,
                        revision_local: status.revision_local,
                        revision_local_number: status.revision_local_number,
                        revision_remote: status.revision_remote,
                        revision_remote_number: status.revision_remote_number,
                        is_local_ahead: status.is_local_ahead != 0,
                        is_remote_ahead: status.is_remote_ahead != 0,
                        remote_available: status.remote_available != 0,
                        remote_authorized: status.remote_authorized != 0,
                        remote_branch_exist: status.remote_branch_exist != 0,
                    }
                }
                LORE_EVENT_REPOSITORY_STATUS_FILE => {
                    let file = &data.repository_status_file;
                    Self::RepositoryStatusFile {
                        path: file.path.try_to_str()?,
                        size: file.size,
                        action: file.action,
                        kind: file.type_,
                        staged: file.flag_staged != 0,
                        merged: file.flag_merged != 0,
                        conflict: file.flag_conflict != 0,
                        conflict_unresolved: file.flag_conflict_unresolved != 0,
                        conflict_automerged: file.flag_conflict_automerged != 0,
                        conflict_mine: file.flag_conflict_mine != 0,
                        conflict_theirs: file.flag_conflict_theirs != 0,
                        dirty: file.flag_dirty != 0,
                        from_path: file.from_path.try_to_str()?,
                    }
                }
                LORE_EVENT_REPOSITORY_STATUS_COUNT => Self::RepositoryStatusCount {
                    directories: data.repository_status_count.directories,
                    files: data.repository_status_count.files,
                },
                LORE_EVENT_REPOSITORY_STATUS_SUMMARY => Self::RepositoryStatusSummary {
                    adds: data.repository_status_summary.adds,
                    deletes: data.repository_status_summary.deletes,
                    modifies: data.repository_status_summary.modifies,
                    moves: data.repository_status_summary.moves,
                    copies: data.repository_status_summary.copies,
                },
                LORE_EVENT_REPOSITORY_STATE_DUMP_NODE => Self::RepositoryStateDumpNode {
                    path: data.repository_state_dump_node.name.try_to_str()?,
                },
                LORE_EVENT_REVISION_COMMIT_END => Self::RevisionCommitEnd {
                    count: data.revision_commit_end.count,
                },
                LORE_EVENT_REVISION_COMMIT_REVISION => Self::RevisionCommitRevision {
                    repository: data.revision_commit_revision.repository,
                    branch: data.revision_commit_revision.branch,
                    revision: data.revision_commit_revision.revision,
                    revision_number: data.revision_commit_revision.revision_number,
                    parents: [
                        data.revision_commit_revision.parent,
                        data.revision_commit_revision.parent_other,
                    ],
                },
                LORE_EVENT_REVISION_HISTORY => Self::RevisionHistory {
                    repository: data.revision_history.repository,
                    branch: data.revision_history.branch,
                },
                LORE_EVENT_REVISION_HISTORY_ENTRY => Self::RevisionHistoryEntry {
                    revision: data.revision_history_entry.revision,
                    revision_number: data.revision_history_entry.revision_number,
                    parents: data.revision_history_entry.parent,
                },
                LORE_EVENT_REVISION_SYNC_REVISION => Self::RevisionSyncRevision {
                    branch: data.revision_sync_revision.branch,
                    revision: data.revision_sync_revision.revision,
                    revision_number: data.revision_sync_revision.revision_number,
                    merge: data.revision_sync_revision.flag_merge != 0,
                    conflict: data.revision_sync_revision.flag_conflict != 0,
                },
                LORE_EVENT_REVISION_SYNC_TARGET => Self::RevisionSyncTarget {
                    remote: data.revision_sync_target.remote.try_to_str()?,
                    repository: data.revision_sync_target.repository,
                    branch: data.revision_sync_target.branch,
                    branch_name: data.revision_sync_target.branch_name.try_to_str()?,
                    source_revision: data.revision_sync_target.source_revision,
                    source_revision_number: data.revision_sync_target.source_revision_number,
                    target_revision: data.revision_sync_target.target_revision,
                    target_revision_number: data.revision_sync_target.target_revision_number,
                    is_latest: data.revision_sync_target.is_latest != 0,
                    local: data.revision_sync_target.local != 0,
                },
                LORE_EVENT_REVISION_INFO => Self::RevisionInfo {
                    repository: data.revision_info.repository,
                    revision: data.revision_info.revision,
                    revision_number: data.revision_info.revision_number,
                    parents: data.revision_info.parent,
                },
                LORE_EVENT_REVISION_TREE_LOADED => Self::RevisionTreeLoaded {
                    handle_id: data.revision_tree_loaded.handle_id,
                },
                LORE_EVENT_REVISION_TREE_RESOLVE_PATH_COMPLETE => {
                    Self::RevisionTreeResolvePathComplete {
                        id: data.revision_tree_resolve_path_complete.id,
                        node_id: data.revision_tree_resolve_path_complete.node_id,
                        repository: data.revision_tree_resolve_path_complete.repository,
                        revision: data.revision_tree_resolve_path_complete.revision,
                        error_code: data.revision_tree_resolve_path_complete.error_code,
                    }
                }
                LORE_EVENT_REVISION_TREE_LIST_CHILDREN_BEGIN => {
                    Self::RevisionTreeListChildrenBegin {
                        id: data.revision_tree_list_children_begin.id,
                        repository: data.revision_tree_list_children_begin.repository,
                        revision: data.revision_tree_list_children_begin.revision,
                        error_code: data.revision_tree_list_children_begin.error_code,
                    }
                }
                LORE_EVENT_REVISION_TREE_CHILD => Self::RevisionTreeChild {
                    id: data.revision_tree_child.id,
                    node_id: data.revision_tree_child.node_id,
                    name: data.revision_tree_child.name.try_to_str()?,
                    parent_id: data.revision_tree_child.parent_id,
                    kind: data.revision_tree_child.kind,
                    mode: data.revision_tree_child.mode,
                    size: data.revision_tree_child.size,
                    address: data.revision_tree_child.address,
                    error_code: data.revision_tree_child.error_code,
                },
                LORE_EVENT_REVISION_TREE_NODE_INFO => Self::RevisionTreeNodeInfo {
                    id: data.revision_tree_node_info.id,
                    node_id: data.revision_tree_node_info.node_id,
                    repository: data.revision_tree_node_info.repository,
                    revision: data.revision_tree_node_info.revision,
                    name: data.revision_tree_node_info.name.try_to_str()?,
                    parent_id: data.revision_tree_node_info.parent_id,
                    kind: data.revision_tree_node_info.kind,
                    mode: data.revision_tree_node_info.mode,
                    size: data.revision_tree_node_info.size,
                    address: data.revision_tree_node_info.address,
                    file_id: data.revision_tree_node_info.file_id,
                    error_code: data.revision_tree_node_info.error_code,
                },
                LORE_EVENT_REVISION_TREE_NODE_PATH => Self::RevisionTreeNodePath {
                    id: data.revision_tree_node_path.id,
                    repository: data.revision_tree_node_path.repository,
                    revision: data.revision_tree_node_path.revision,
                    path: data.revision_tree_node_path.path.try_to_str()?,
                    error_code: data.revision_tree_node_path.error_code,
                },
                LORE_EVENT_REVISION_TREE_INFO => Self::RevisionTreeInfo {
                    id: data.revision_tree_info.id,
                    repository: data.revision_tree_info.repository,
                    revision: data.revision_tree_info.revision,
                    parents: data.revision_tree_info.parent,
                    creation_timestamp: data.revision_tree_info.creation_timestamp,
                    author_identity: data.revision_tree_info.author_identity.try_to_str()?,
                    metadata_key_count: data.revision_tree_info.metadata_key_count,
                    error_code: data.revision_tree_info.error_code,
                },
                LORE_EVENT_REVISION_TREE_CLOSE_COMPLETE => Self::RevisionTreeCloseComplete {
                    id: data.revision_tree_close_complete.id,
                    error_code: data.revision_tree_close_complete.error_code,
                },
                LORE_EVENT_STORAGE_OPENED => Self::StorageOpened {
                    handle_id: data.storage_opened.handle_id,
                },
                LORE_EVENT_STORAGE_PUT_ITEM_COMPLETE => Self::StoragePutItemComplete {
                    id: data.storage_put_item_complete.id,
                    address: data.storage_put_item_complete.address,
                    error_code: data.storage_put_item_complete.error_code,
                },
                LORE_EVENT_STORAGE_GET_HEADER => Self::StorageGetHeader {
                    id: data.storage_get_header.id,
                    address: data.storage_get_header.address,
                    size_content: data.storage_get_header.size_content,
                },
                LORE_EVENT_STORAGE_GET_DATA => Self::StorageGetData {
                    id: data.storage_get_data.id,
                    offset: data.storage_get_data.offset,
                    bytes: std::slice::from_raw_parts(
                        data.storage_get_data.bytes.ptr.cast::<u8>(),
                        data.storage_get_data.bytes.len,
                    ),
                },
                LORE_EVENT_STORAGE_GET_ITEM_COMPLETE => Self::StorageGetItemComplete {
                    id: data.storage_get_item_complete.id,
                    address: data.storage_get_item_complete.address,
                    error_code: data.storage_get_item_complete.error_code,
                },
                LORE_EVENT_STORAGE_GET_METADATA_ITEM_COMPLETE => {
                    Self::StorageGetMetadataItemComplete {
                        id: data.storage_get_metadata_item_complete.id,
                        address: data.storage_get_metadata_item_complete.address,
                        fragment: data.storage_get_metadata_item_complete.fragment,
                        error_code: data.storage_get_metadata_item_complete.error_code,
                    }
                }
                _ => Self::Other { tag },
            })
        }
    }
}

/// Forwards an [`Event::Log`] to the `log` crate, ignoring anything else. Call
/// it from a callback to report what Lore says while a call runs; the events a
/// callback does not match on are otherwise dropped.
pub fn log_event(event: &Result<Event<'_>, std::str::Utf8Error>) {
    let Ok(Event::Log { level, message }) = event else {
        return;
    };

    let level = match *level {
        LORE_LOG_LEVEL_ERROR => log::Level::Error,
        LORE_LOG_LEVEL_WARN => log::Level::Warn,
        LORE_LOG_LEVEL_INFO => log::Level::Info,
        LORE_LOG_LEVEL_TRACE => log::Level::Trace,
        _ => log::Level::Debug,
    };

    log::log!(target: "lore", level, "{message}");
}
