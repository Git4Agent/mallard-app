/** Mallard Cloud furniture: chip, panel, scopes, Publish version (managed-cloud/). */
export default {
  /** The chip's one word for whatever the service answered. */
  word: {
    checking: "Checking…",
    signIn: "Sign in",
    noAccess: "No access",
    cantCheck: "Can't check",
    beingDeleted: "Being deleted",
    gone: "Gone",
    rejected: "Rejected",
    inReview: "In review",
    notPushed: "Not pushed",
    cantCompare: "Can't compare",
    private: "Private",
    draft: "Draft",
    publicVersion: "Public v{{version}}",
  },
  /** The line under the chip's word: where the draft stands. Backend
   *  sentences replace these when the service sent one. */
  line: {
    checking: "Checking Mallard Cloud…",
    signedOut: "Signed out of Mallard Cloud.",
    refused: "Mallard Cloud refused this account.",
    couldNotCheck: "Could not check Mallard Cloud.",
    pushCreates: "Push creates it again.",
    rejected: "The release request was rejected.",
    waitingOwner: "Waiting for the Owner.",
    draftWaitingOwner: "The draft is waiting for the Owner.",
    versionWaitingOwner: "Version {{version}} is waiting for the Owner.",
    waitingAdmin: "Waiting for a Mallard Cloud admin.",
    draftWaitingAdmin: "The draft is waiting for a Mallard Cloud admin.",
    versionWaitingAdmin: "Version {{version}} is waiting for a Mallard Cloud admin.",
    nothingPushed: "Nothing pushed yet.",
    cantCompare: "Couldn't compare the draft with the versions.",
    draftOnly: "Draft only. No version yet.",
    privateAhead_one: "The draft is {{count}} push past version {{version}}.",
    privateAhead_other: "The draft is {{count}} pushes past version {{version}}.",
    privateVersion: "Version {{version}}. Collaborators only.",
    publicAhead_one: "{{count}} push past version {{version}}, which is public.",
    publicAhead_other: "{{count}} pushes past version {{version}}, which is public.",
    publicAheadOther_one: "{{count}} push past version {{version}}. Public is version {{shown}}.",
    publicAheadOther_other: "{{count}} pushes past version {{version}}. Public is version {{shown}}.",
    notPublic: "Not public. Public is version {{shown}}.",
    public: "Version {{version}} is public. The draft is the same.",
  },
  /** The workspace's note for a repository that is not ready. */
  notice: {
    beingDeleted: "Being deleted",
    repositoryGone: "Repository gone",
    signInRequired: "Sign-in required",
    repositoryUnavailable: "Repository unavailable",
    statusUnavailable: "Status unavailable",
    /** A gone repository's line follows its title. */
    goneMessage: "— {{message}}",
  },
  /** Why Publish or Discard draft is off, for the row's title. */
  hold: {
    nothingPushed: "Nothing pushed yet",
    nothingToRemove: "Nothing to remove: the draft is version {{version}}",
    nothingNew: "Nothing new since version {{version}}",
  },
  /** What Discard draft asks before it sends anything. */
  discard: {
    title: "Discard draft?",
    message: "Discard the draft of “{{address}}”?\n\n{{removes}}\n\nNothing is deleted on this computer. This cannot be undone.",
    removesAfter: "Removes every push after version {{version}} from Mallard Cloud. The draft returns to version {{version}}. Every version stays.",
    removesAfterWithdrawn: "Removes every push after version {{version}} from Mallard Cloud. The draft returns to version {{version}}. Every version stays. The open release request is withdrawn.",
    removesAll: "Removes every push from Mallard Cloud. There is no version to return to: the repository stays, empty.",
    removesAllWithdrawn: "Removes every push from Mallard Cloud. There is no version to return to: the repository stays, empty. The open release request is withdrawn.",
    ok: "Discard",
    /** The version the draft returns to, in words. */
    kept: "version {{version}}",
  },
  /** The four facts under the panel's headline. */
  fact: {
    draft: "Draft",
    version: "Version",
    public: "Public",
    you: "You",
    notPublished: "Not published",
    ahead_one: "{{count}} push on",
    ahead_other: "{{count}} pushes on",
    sameAs: "Same as {{version}}",
    private: "Private",
  },
  /** The account's standing on a repository. */
  role: {
    owner: "Owner",
    admin: "Admin",
    member: "Member",
    collaborator: "Collaborator",
    contributor: "Contributor",
  },
  /** Who reads a repository. */
  visibility: {
    private: "Private",
    public: "Public",
  },
  /** The panel under the chip. */
  panel: {
    publishVersion: "Publish version {{version}}…",
    requestRelease: "Request public release",
    viewRelease: "View release request",
    viewDraft: "View draft",
    versions: "Versions",
    viewPublic: "View public version",
    discardDraft: "Discard draft…",
    discardBackTo: "Discard draft… · back to version {{version}}",
    discardEveryPush: "Discard draft… · every push",
    contributor: "Not a collaborator: anyone signed in can push here, and the Owner decides what's released.",
    storageSettings: "Storage settings",
    retry: "Retry",
    actions: "Mallard Cloud actions",
    copyFailed: "Couldn't copy the link.",
    copyFailedLine: "Couldn't copy. The address is in the button's tooltip.",
  },
  /** Each link row's copy button, and what it says once it copied. */
  copy: {
    release: "Copy release link",
    draft: "Copy draft link",
    versions: "Copy versions link",
    public: "Copy public link",
  },
  copied: {
    release: "Copied the release link.",
    draft: "Copied the draft link.",
    versions: "Copied the versions link.",
    public: "Copied the public link.",
  },
  chip: {
    name: "Mallard Cloud: {{word}}. {{line}}",
    /** Before the first answer: the chip's word and line said once. */
    checking: "Mallard Cloud: Checking…. Checking Mallard Cloud…",
  },
  /** The sheet Publish version asks in. */
  publish: {
    title: "Publish version {{version}}",
    close: "Close without publishing",
    notPushed_one: "{{count}} session here is not pushed, so not in it",
    notPushed_other: "{{count}} sessions here are not pushed, so not in it",
    nameLabel: "Name, optional",
    namePlaceholder: "Name (optional)",
    publishing: "Publishing…",
  },
  /** A settled Mallard Cloud storage's website band. */
  address: {
    label: "Mallard Cloud website",
    open: "Open {{origin}} in your browser",
    copy: "Copy {{origin}}",
    openWebsite: "Open website",
    signedInAs: "Signed in as <b><login/></b>",
    copyFailed: "Couldn't copy the address.",
    copied: "Copied the address.",
  },
  /** The one field that filters, looks up an address, or joins with a code. */
  lookup: {
    label: "Search, address or invitation code",
    placeholder: "Search, or paste a link, owner/name or an invitation code",
    joining: "Joining…",
    looking: "Looking up…",
    join: "Join",
    lookUp: "Look up",
    joinHint: "Join to become a member.",
  },
  /** The scope tabs, the listings and their rows. */
  scopes: {
    tabs: "Where to look",
    sort: "Sort",
    newest: "Newest",
    mostStars: "Most stars",
    onGitHub: "On GitHub",
    lookingUp: "Looking up…",
    countIn: "{{count}} in {{scope}}",
    browsePublic: "Browse Public",
    showMore: "Show more",
    notPushedYet: "not pushed yet",
    public: "Public",
    publicRole: "Public · {{role}}",
    refresh: "Refresh",
    refreshing: "Refreshing…",
    updated: "Updated",
  },
  /** The address a New row previews, refused. */
  name: {
    refusalProject: "Name needs a letter or digit; rename the project first.",
    refusalItem: "Name needs a letter or digit; rename the item first.",
  },
} as const;
