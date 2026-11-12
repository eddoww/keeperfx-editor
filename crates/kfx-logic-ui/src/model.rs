//! Document model of level logic, independent of script syntax.
//!
//! A level script is a sequence of [`Node`]s:
//!
//! * [`Event`]: a trigger (zero or more [`Condition`]s, combined with AND by nesting) plus an
//!   ordered body of [`Action`]s and nested events. An event without conditions runs while the
//!   level loads ("on level start"). Nested IF blocks with nothing between them are shown as one
//!   event with several conditions; they are written back as nested IFs.
//! * [`Party`]: `CREATE_PARTY` plus its `ADD_TO_PARTY` members, defined at load time and spawned
//!   by actions anywhere.
//!
//! Semantics (KeeperFX source): a condition is re-evaluated every turn
//! (`process_condition`, `src/lvl_script_conditions.c:463`) and only while its parent condition is
//! active (`condition_inactive`, `:446`), so nested IFs mean AND. A command under a condition
//! runs once, the first turn the condition is met, unless `NEXT_COMMAND_REUSABLE` came right
//! before it: then it runs every turn the condition holds (`process_values`,
//! `src/lvl_script.c:1128`, `TrgF_REUSABLE`; the flag lives for exactly one line,
//! `script_scan_line`, `src/lvl_script.c:733`).
//!
//! Every node remembers the source line it came from (`raw`) and is written back verbatim until
//! it is edited, so an unedited script exports byte for byte. Comment and blank lines are kept
//! in `pre` (lines before the node) and `tail` (lines before an `ENDIF` / end of file).

/// Stable identity of a node inside one document (used by the UI and the host callbacks).
pub type NodeId = u64;

/// A command as written: upper-case name and the source text of each argument (strings keep
/// their quotes, `RANDOM(...)` is kept whole).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Cmd {
    pub name: String,
    pub args: Vec<String>,
}

impl Cmd {
    pub fn new(name: &str, args: &[&str]) -> Cmd {
        Cmd {
            name: name.to_ascii_uppercase(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }
    pub fn arg(&self, i: usize) -> &str {
        self.args.get(i).map(|s| s.as_str()).unwrap_or("")
    }
    /// Argument text with string quotes removed.
    pub fn arg_unquoted(&self, i: usize) -> &str {
        let a = self.arg(i);
        a.strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(a)
    }
    pub fn set_arg(&mut self, i: usize, v: impl Into<String>) {
        while self.args.len() <= i {
            self.args.push(String::new());
        }
        self.args[i] = v.into();
    }
}

/// One script command executed by an event.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub id: NodeId,
    /// Comment / blank lines right before this action (verbatim, without newline).
    pub pre: Vec<String>,
    pub cmd: Cmd,
    /// `NEXT_COMMAND_REUSABLE` precedes it: runs every turn its condition holds.
    pub reusable: bool,
    /// The original `NEXT_COMMAND_REUSABLE` line, written back while `reusable` is unchanged.
    pub reusable_raw: Option<String>,
    /// Verbatim source line while unedited.
    pub raw: Option<String>,
    /// Trailing `REM` comment of the line.
    pub trail: Option<String>,
    /// The line is not a command the editor can type (unknown command, syntax error, or text
    /// that would not survive regeneration): only `raw` is used and it is edited as text.
    pub opaque: bool,
    /// 1-based line in the imported script (0 for new nodes).
    pub line: usize,
}

/// One `IF*` line of an event's trigger.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub id: NodeId,
    /// Comment / blank lines between the previous `IF` of the same event and this one.
    pub pre: Vec<String>,
    pub cmd: Cmd,
    pub raw: Option<String>,
    pub trail: Option<String>,
    /// The `ENDIF` line as written (`None` for a new condition, or one never closed).
    pub endif_raw: Option<String>,
    /// The script never closes this IF (only possible in view-only documents).
    pub unclosed: bool,
    pub line: usize,
    pub endif_line: usize,
}

/// A trigger and what it does.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: NodeId,
    /// Comment / blank lines before the event (its notes).
    pub pre: Vec<String>,
    /// AND-ed conditions, outermost first. Empty: runs at level start.
    pub conditions: Vec<Condition>,
    pub body: Vec<Node>,
    /// Comment / blank lines before the innermost `ENDIF`.
    pub tail: Vec<String>,
    /// Disabled events are written as a `REM kfx-off` block (the game ignores them).
    pub enabled: bool,
    pub line: usize,
}

/// One `ADD_TO_PARTY` member.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub id: NodeId,
    pub pre: Vec<String>,
    pub creature: String,
    pub level: String,
    pub gold: String,
    pub objective: String,
    pub countdown: String,
    pub raw: Option<String>,
    pub trail: Option<String>,
    pub line: usize,
}

/// A named creature group (`CREATE_PARTY`, `src/creature_groups.c`).
#[derive(Clone, Debug, PartialEq)]
pub struct Party {
    pub id: NodeId,
    pub pre: Vec<String>,
    pub name: String,
    pub members: Vec<Member>,
    pub raw: Option<String>,
    pub trail: Option<String>,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Action(Action),
    Event(Event),
    Party(Party),
}

impl Node {
    pub fn id(&self) -> NodeId {
        match self {
            Node::Action(a) => a.id,
            Node::Event(e) => e.id,
            Node::Party(p) => p.id,
        }
    }
    pub fn line(&self) -> usize {
        match self {
            Node::Action(a) => a.line,
            Node::Event(e) => e.line,
            Node::Party(p) => p.line,
        }
    }
    pub fn pre(&self) -> &Vec<String> {
        match self {
            Node::Action(a) => &a.pre,
            Node::Event(e) => &e.pre,
            Node::Party(p) => &p.pre,
        }
    }
    pub fn pre_mut(&mut self) -> &mut Vec<String> {
        match self {
            Node::Action(a) => &mut a.pre,
            Node::Event(e) => &mut e.pre,
            Node::Party(p) => &mut p.pre,
        }
    }
}

/// Which script variable family an alias names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VarFamily {
    Flag,
    Timer,
    CampaignFlag,
}

impl VarFamily {
    pub const ALL: [VarFamily; 3] = [VarFamily::Flag, VarFamily::Timer, VarFamily::CampaignFlag];
    /// Script prefix (`FLAG`, `TIMER`, `CAMPAIGN_FLAG`).
    pub fn prefix(self) -> &'static str {
        match self {
            VarFamily::Flag => "FLAG",
            VarFamily::Timer => "TIMER",
            VarFamily::CampaignFlag => "CAMPAIGN_FLAG",
        }
    }
    /// Number of slots: `flag_desc` FLAG0..FLAG7, `timer_desc` TIMER0..TIMER7,
    /// `campaign_flag_desc` CAMPAIGN_FLAG0..CAMPAIGN_FLAG7 (`src/lvl_script_commands.c`).
    pub fn slots(self) -> u32 {
        let t = kfx_script::Tables::get();
        let n = match self {
            VarFamily::Flag => t.named("flag_desc").len(),
            VarFamily::Timer => t.named("timer_desc").len(),
            VarFamily::CampaignFlag => t.named("campaign_flag_desc").len(),
        };
        if n == 0 {
            8
        } else {
            n as u32
        }
    }
    /// Parse `FLAG3` / `TIMER0` / `CAMPAIGN_FLAG7`.
    pub fn parse(var: &str) -> Option<(VarFamily, u32)> {
        let v = var.to_ascii_uppercase();
        for f in [VarFamily::CampaignFlag, VarFamily::Flag, VarFamily::Timer] {
            if let Some(n) = v.strip_prefix(f.prefix()) {
                if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) {
                    let i: u32 = n.parse().ok()?;
                    if i < f.slots() {
                        return Some((f, i));
                    }
                }
            }
        }
        None
    }
}

/// A named alias for `FLAGn` / `TIMERn` / `CAMPAIGN_FLAGn` of one player. Stored in the script as
/// `REM kfx-var NAME = PLAYER0.FLAG1` lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarAlias {
    pub name: String,
    pub player: String,
    pub family: VarFamily,
    pub index: u32,
}

impl VarAlias {
    pub fn var(&self) -> String {
        format!("{}{}", self.family.prefix(), self.index)
    }
}

/// Marker of a disabled event's lines.
pub const OFF_MARK: &str = "REM kfx-off ";
/// Marker of a variable alias line.
pub const VAR_MARK: &str = "REM kfx-var ";

/// The level logic of one DK script.
#[derive(Clone, Debug, PartialEq)]
pub struct LogicDoc {
    pub nodes: Vec<Node>,
    /// Comment / blank lines at the end of the file.
    pub tail: Vec<String>,
    pub variables: Vec<VarAlias>,
    /// The file ended with a newline.
    pub final_newline: bool,
    /// Line ending for new lines (`"\n"` or `"\r\n"`, detected on import).
    pub crlf: bool,
    /// `LEVEL_VERSION` in effect (0: the DK1 command set).
    pub version: i64,
    /// Set when the script cannot be represented faithfully: the editor then only shows it.
    pub read_only: Option<String>,
    pub(crate) next_id: NodeId,
}

impl Default for LogicDoc {
    fn default() -> Self {
        LogicDoc {
            nodes: vec![],
            tail: vec![],
            variables: vec![],
            final_newline: true,
            crlf: false,
            version: 1,
            read_only: None,
            next_id: 1,
        }
    }
}

impl LogicDoc {
    pub fn new_id(&mut self) -> NodeId {
        let i = self.next_id;
        self.next_id += 1;
        i
    }

    /// New action node (not yet inserted).
    pub fn new_action(&mut self, cmd: Cmd) -> Action {
        Action {
            id: self.new_id(),
            pre: vec![],
            cmd,
            reusable: false,
            reusable_raw: None,
            raw: None,
            trail: None,
            opaque: false,
            line: 0,
        }
    }

    pub fn new_condition(&mut self, cmd: Cmd) -> Condition {
        Condition {
            id: self.new_id(),
            pre: vec![],
            cmd,
            raw: None,
            trail: None,
            endif_raw: None,
            unclosed: false,
            line: 0,
            endif_line: 0,
        }
    }

    pub fn new_event(&mut self, conditions: Vec<Condition>, body: Vec<Node>) -> Event {
        Event {
            id: self.new_id(),
            pre: vec![],
            conditions,
            body,
            tail: vec![],
            enabled: true,
            line: 0,
        }
    }

    pub fn new_party(&mut self, name: &str) -> Party {
        Party {
            id: self.new_id(),
            pre: vec![],
            name: name.to_ascii_uppercase(),
            members: vec![],
            raw: None,
            trail: None,
            line: 0,
        }
    }

    pub fn new_member(
        &mut self,
        creature: &str,
        level: u32,
        gold: u32,
        objective: &str,
        countdown: u32,
    ) -> Member {
        Member {
            id: self.new_id(),
            pre: vec![],
            creature: creature.to_ascii_uppercase(),
            level: level.to_string(),
            gold: gold.to_string(),
            objective: objective.to_ascii_uppercase(),
            countdown: countdown.to_string(),
            raw: None,
            trail: None,
            line: 0,
        }
    }

    /// Re-number every node (after pasting nodes copied from another document).
    pub fn renumber(&mut self) {
        fn walk(nodes: &mut [Node], d: &mut NodeId) {
            for n in nodes {
                match n {
                    Node::Action(a) => {
                        a.id = *d;
                        *d += 1;
                    }
                    Node::Party(p) => {
                        p.id = *d;
                        *d += 1;
                        for m in &mut p.members {
                            m.id = *d;
                            *d += 1;
                        }
                    }
                    Node::Event(e) => {
                        e.id = *d;
                        *d += 1;
                        for c in &mut e.conditions {
                            c.id = *d;
                            *d += 1;
                        }
                        walk(&mut e.body, d);
                    }
                }
            }
        }
        let mut d = 1;
        walk(&mut self.nodes, &mut d);
        self.next_id = d;
    }

    /// Give fresh ids to a node tree (duplicate / paste).
    pub fn reid(&mut self, node: &mut Node) {
        match node {
            Node::Action(a) => a.id = self.new_id(),
            Node::Party(p) => {
                p.id = self.new_id();
                for m in &mut p.members {
                    m.id = self.new_id();
                }
            }
            Node::Event(e) => {
                e.id = self.new_id();
                for c in &mut e.conditions {
                    c.id = self.new_id();
                }
                for n in &mut e.body {
                    self.reid(n);
                }
            }
        }
    }

    /// Every top-level and nested event, depth first, with its nesting depth.
    pub fn events(&self) -> Vec<(&Event, usize)> {
        fn walk<'a>(nodes: &'a [Node], depth: usize, out: &mut Vec<(&'a Event, usize)>) {
            for n in nodes {
                if let Node::Event(e) = n {
                    out.push((e, depth));
                    walk(&e.body, depth + 1, out);
                }
            }
        }
        let mut out = vec![];
        walk(&self.nodes, 0, &mut out);
        out
    }

    pub fn parties(&self) -> impl Iterator<Item = &Party> {
        self.nodes.iter().filter_map(|n| {
            if let Node::Party(p) = n {
                Some(p)
            } else {
                None
            }
        })
    }

    pub fn party(&self, name: &str) -> Option<&Party> {
        self.parties().find(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn party_mut(&mut self, id: NodeId) -> Option<&mut Party> {
        self.nodes.iter_mut().find_map(|n| match n {
            Node::Party(p) if p.id == id => Some(p),
            _ => None,
        })
    }

    /// The event with `id` anywhere in the tree.
    pub fn event(&self, id: NodeId) -> Option<&Event> {
        self.events()
            .into_iter()
            .map(|(e, _)| e)
            .find(|e| e.id == id)
    }

    pub fn event_mut(&mut self, id: NodeId) -> Option<&mut Event> {
        fn walk(nodes: &mut [Node], id: NodeId) -> Option<&mut Event> {
            for n in nodes {
                if let Node::Event(e) = n {
                    if e.id == id {
                        return Some(e);
                    }
                    if let Some(f) = walk(&mut e.body, id) {
                        return Some(f);
                    }
                }
            }
            None
        }
        walk(&mut self.nodes, id)
    }

    pub fn action_mut(&mut self, id: NodeId) -> Option<&mut Action> {
        fn walk(nodes: &mut [Node], id: NodeId) -> Option<&mut Action> {
            for n in nodes {
                match n {
                    Node::Action(a) if a.id == id => return Some(a),
                    Node::Event(e) => {
                        if let Some(f) = walk(&mut e.body, id) {
                            return Some(f);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        walk(&mut self.nodes, id)
    }

    pub fn condition_mut(&mut self, id: NodeId) -> Option<&mut Condition> {
        fn walk(nodes: &mut [Node], id: NodeId) -> Option<&mut Condition> {
            for n in nodes {
                if let Node::Event(e) = n {
                    if let Some(c) = e.conditions.iter_mut().find(|c| c.id == id) {
                        return Some(c);
                    }
                    if let Some(f) = walk(&mut e.body, id) {
                        return Some(f);
                    }
                }
            }
            None
        }
        walk(&mut self.nodes, id)
    }

    /// Remove the node with `id` from wherever it is; returns it.
    pub fn remove(&mut self, id: NodeId) -> Option<Node> {
        fn walk(nodes: &mut Vec<Node>, id: NodeId) -> Option<Node> {
            if let Some(i) = nodes.iter().position(|n| n.id() == id) {
                return Some(nodes.remove(i));
            }
            for n in nodes.iter_mut() {
                if let Node::Event(e) = n {
                    if let Some(f) = walk(&mut e.body, id) {
                        return Some(f);
                    }
                }
            }
            None
        }
        walk(&mut self.nodes, id)
    }

    /// The list containing `id` and its index there (`None` parent = top level).
    pub fn locate(&self, id: NodeId) -> Option<(Option<NodeId>, usize)> {
        fn walk(
            nodes: &[Node],
            parent: Option<NodeId>,
            id: NodeId,
        ) -> Option<(Option<NodeId>, usize)> {
            if let Some(i) = nodes.iter().position(|n| n.id() == id) {
                return Some((parent, i));
            }
            for n in nodes {
                if let Node::Event(e) = n {
                    if let Some(f) = walk(&e.body, Some(e.id), id) {
                        return Some(f);
                    }
                }
            }
            None
        }
        walk(&self.nodes, None, id)
    }

    /// Mutable access to the list with parent `parent` (`None` = top level).
    pub fn list_mut(&mut self, parent: Option<NodeId>) -> Option<&mut Vec<Node>> {
        match parent {
            None => Some(&mut self.nodes),
            Some(p) => self.event_mut(p).map(|e| &mut e.body),
        }
    }

    /// Insert `node` into the list of `parent` at `index` (clamped).
    pub fn insert(&mut self, parent: Option<NodeId>, index: usize, node: Node) -> bool {
        match self.list_mut(parent) {
            Some(l) => {
                let i = index.min(l.len());
                l.insert(i, node);
                true
            }
            None => false,
        }
    }

    /// Forget the source text of every typed node, so export regenerates all lines in the
    /// canonical form (opaque lines stay verbatim). Comments are kept.
    pub fn forget_source(&mut self) {
        fn walk(nodes: &mut [Node]) {
            for n in nodes {
                match n {
                    Node::Action(a) => {
                        a.touch();
                        a.reusable_raw = None;
                    }
                    Node::Party(p) => {
                        p.raw = None;
                        for m in &mut p.members {
                            m.raw = None;
                        }
                    }
                    Node::Event(e) => {
                        for c in &mut e.conditions {
                            c.raw = None;
                            c.endif_raw = None;
                        }
                        walk(&mut e.body);
                    }
                }
            }
        }
        walk(&mut self.nodes);
    }

    /// Number of opaque (verbatim) actions.
    pub fn opaque_count(&self) -> usize {
        fn walk(nodes: &[Node]) -> usize {
            nodes
                .iter()
                .map(|n| match n {
                    Node::Action(a) => a.opaque as usize,
                    Node::Event(e) => walk(&e.body),
                    Node::Party(_) => 0,
                })
                .sum()
        }
        walk(&self.nodes)
    }

    /// Alias for `(player, var)`, if declared.
    pub fn alias_of(&self, player: &str, var: &str) -> Option<&VarAlias> {
        self.variables
            .iter()
            .find(|v| v.player.eq_ignore_ascii_case(player) && v.var().eq_ignore_ascii_case(var))
    }
}

/// Mark an edited node: it is regenerated from its fields on export.
pub trait Dirty {
    fn touch(&mut self);
}

impl Dirty for Action {
    fn touch(&mut self) {
        if !self.opaque {
            self.raw = None;
        }
    }
}
impl Dirty for Condition {
    fn touch(&mut self) {
        self.raw = None;
    }
}
impl Dirty for Member {
    fn touch(&mut self) {
        self.raw = None;
    }
}
impl Dirty for Party {
    fn touch(&mut self) {
        self.raw = None;
    }
}
