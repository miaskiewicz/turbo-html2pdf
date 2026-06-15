//! Tagged / accessible PDF (PDF/UA-1, AC-11.1) — only compiled under the
//! `pdf-ua` feature.
//!
//! Two cooperating walks build a tagged document:
//!
//! * The **painter** (`page.rs`) brackets each painted fragment in a marked
//!   content sequence: real content (text, images) gets a per-page `/MCID`,
//!   while decoration (box backgrounds/borders, the watermark, running
//!   header/footer chrome) is marked `/Artifact` so assistive tech skips it.
//! * The **planner** here ([`UaPlan`]) walks the same fragments in the same
//!   order to build a `StructTreeRoot`: a tree of `StructElem`s mirroring the
//!   semantic HTML (`Document` → headings / paragraphs / lists / tables /
//!   figures), with every `/MCID` linked back to its element through a
//!   `/ParentTree`. Reading order is document order.
//!
//! The two walks must assign MCIDs identically; both rely on [`wrap_kind`] and
//! the band split ([`super::page`]) to decide what is content vs. artifact, so a
//! given page yields the same MCID sequence in both passes.

use pdf_writer::types::StructRole;
use pdf_writer::{Content, Finish, Name, Pdf, Ref, TextStr};

use crate::layout::fragment::{Fragment, FragmentContent, UaRole};
use crate::layout::value::BorderEdges;
use crate::paginate::Page;

use super::EmitOptions;

// --------------------------------------------------------------------------
// content-stream marked content
// --------------------------------------------------------------------------

/// How a fragment's paint is bracketed in the content stream.
pub(super) enum WrapKind {
    /// Not bracketed (paints nothing, or is a directive).
    None,
    /// Bracketed as `/Artifact … BDC … EMC` (decoration, skipped by readers).
    Artifact,
    /// Bracketed as real content with an `/MCID` (linked into the struct tree).
    Content,
}

/// Decide how a fragment's own paint is wrapped. Real text/images in the body
/// (and footnote) bands are content; box decoration and everything in an
/// artifact band is an artifact; a box that paints nothing needs no bracket.
pub(super) fn wrap_kind(frag: &Fragment, artifact_band: bool) -> WrapKind {
    if artifact_band {
        return artifact_or_none(frag);
    }
    content_band_kind(frag)
}

/// The wrap for a fragment in a content band: real text/images are tagged
/// content; a painting box is decoration; everything else is unbracketed.
fn content_band_kind(frag: &Fragment) -> WrapKind {
    match &frag.content {
        FragmentContent::TextLine { .. } | FragmentContent::Image(_) => WrapKind::Content,
        FragmentContent::Box { background, border } => box_wrap(*background, border),
        FragmentContent::Directive(_) => WrapKind::None,
    }
}

/// A box's wrap: an artifact when it paints (background or border), else nothing.
fn box_wrap(background: Option<crate::layout::value::Rgba>, border: &BorderEdges) -> WrapKind {
    if background.is_some() || border.any_visible() {
        WrapKind::Artifact
    } else {
        WrapKind::None
    }
}

/// In an artifact band, a painting fragment is an artifact; an empty one needs
/// no bracket.
fn artifact_or_none(frag: &Fragment) -> WrapKind {
    if paints(frag) {
        WrapKind::Artifact
    } else {
        WrapKind::None
    }
}

/// Whether a fragment emits any paint operators at all (so a marked-content
/// bracket would wrap real bytes).
fn paints(frag: &Fragment) -> bool {
    match &frag.content {
        FragmentContent::TextLine { .. } | FragmentContent::Image(_) => true,
        FragmentContent::Box { background, border } => background.is_some() || border.any_visible(),
        FragmentContent::Directive(_) => false,
    }
}

/// Begin an `/Artifact BDC` marked content sequence.
pub(super) fn begin_artifact(content: &mut Content) {
    content.begin_marked_content(Name(b"Artifact"));
}

/// Begin a `/P <</MCID n>> BDC` marked content sequence. The tag is always `/P`
/// in the content stream; the real role lives on the struct element the MCID
/// resolves to via the parent tree.
pub(super) fn begin_mcid(content: &mut Content, mcid: i32) {
    let mut mc = content.begin_marked_content_with_properties(Name(b"P"));
    mc.properties().identify(mcid);
}

/// Hands the painter the next MCID for a page, in paint order. The planner
/// assigned `0, 1, 2, …` in the same order, so a bare counter — capped at the
/// page's content count — suffices.
pub(super) struct Marker {
    next: i32,
    count: i32,
}

impl Marker {
    pub(super) fn new(tags: &PageTags) -> Marker {
        Marker {
            next: 0,
            count: tags.mcid_count,
        }
    }

    /// The next MCID for this page. Never exceeds the planned count: the painter
    /// and planner walk the same fragments in the same order.
    pub(super) fn next(&mut self) -> i32 {
        debug_assert!(self.next < self.count, "more painted MCIDs than planned");
        let id = self.next;
        self.next += 1;
        id
    }
}

/// Per-page marked-content info handed to the painter: the count of content
/// MCIDs the planner assigned for the page (the painter numbers them `0..count`
/// in the same paint order).
pub(super) struct PageTags {
    pub(super) mcid_count: i32,
}

// --------------------------------------------------------------------------
// structure tree planning
// --------------------------------------------------------------------------

/// One node in the in-memory structure tree, before it is written out.
struct StructNode {
    role: StructRole,
    /// `/Alt` text for a figure, else `None`.
    alt: Option<String>,
    /// The 0-based page this element's content lives on (its `/Pg`).
    page: usize,
    /// Indices of child struct nodes, in reading order.
    children: Vec<usize>,
    /// MCIDs of this element's own marked content, on its page, in order.
    mcids: Vec<i32>,
}

/// The whole tagged-PDF plan: the struct tree plus the object refs it needs.
pub(super) struct UaPlan {
    nodes: Vec<StructNode>,
    /// Top-level nodes (the `Document` root's children), in reading order.
    roots: Vec<usize>,
    /// The struct-tree-root object.
    root_ref: Ref,
    /// The `Document` struct element wrapping every page's content.
    doc_ref: Ref,
    /// One object ref per struct node (`nodes[i]` → `node_refs[i]`).
    node_refs: Vec<Ref>,
    /// One parent-tree array object per page (MCID → owning element).
    parent_tree_refs: Vec<Ref>,
    /// The XMP metadata stream object.
    metadata_ref: Ref,
    /// Content MCID count per page, for the painter.
    mcid_counts: Vec<i32>,
}

impl UaPlan {
    /// Build the structure tree from the paginated pages, allocating every object
    /// ref it needs starting at `first`. Returns the plan and the next free id.
    pub(super) fn build(pages: &[Page], first: i32) -> (UaPlan, i32) {
        let mut builder = TreeBuilder::default();
        builder.run(pages);
        UaPlan::allocate(builder, pages.len(), first)
    }

    /// Lay the built tree out over a contiguous block of object ids.
    fn allocate(builder: TreeBuilder, page_count: usize, first: i32) -> (UaPlan, i32) {
        let mut next = first;
        let mut take = || {
            let r = Ref::new(next);
            next += 1;
            r
        };
        let root_ref = take();
        let doc_ref = take();
        let node_refs: Vec<Ref> = builder.nodes.iter().map(|_| take()).collect();
        let parent_tree_refs: Vec<Ref> = (0..page_count).map(|_| take()).collect();
        let metadata_ref = take();
        let plan = UaPlan {
            nodes: builder.nodes,
            roots: builder.roots,
            root_ref,
            doc_ref,
            node_refs,
            parent_tree_refs,
            metadata_ref,
            mcid_counts: builder.mcid_counts,
        };
        (plan, next)
    }

    /// The struct-tree-root object ref (referenced by the catalog).
    pub(super) fn root_ref(&self) -> Ref {
        self.root_ref
    }

    /// The XMP metadata stream ref (referenced by the catalog).
    pub(super) fn metadata_ref(&self) -> Ref {
        self.metadata_ref
    }

    /// The painter inputs for page `i`.
    pub(super) fn page_tags(&self, i: usize) -> PageTags {
        PageTags {
            mcid_count: self.mcid_counts[i],
        }
    }
}

/// Builds the in-memory struct tree by walking pages in reading order.
#[derive(Default)]
struct TreeBuilder {
    nodes: Vec<StructNode>,
    roots: Vec<usize>,
    mcid_counts: Vec<i32>,
}

impl TreeBuilder {
    /// Walk every page's content bands, growing the tree.
    fn run(&mut self, pages: &[Page]) {
        for (page, frags) in pages.iter().enumerate() {
            let mut mcid = 0i32;
            self.walk_band(&frags.body, page, None, &mut mcid);
            self.walk_band(&frags.footnotes, page, None, &mut mcid);
            self.mcid_counts.push(mcid);
        }
    }

    /// Walk a band's top-level fragments. A content fragment with no role
    /// ancestor is given a synthetic paragraph so no MCID lands on the root.
    fn walk_band(
        &mut self,
        frags: &[Fragment],
        page: usize,
        parent: Option<usize>,
        mcid: &mut i32,
    ) {
        for frag in frags {
            self.walk(frag, page, parent, mcid);
        }
    }

    /// Recurse one fragment: open its element (if it has a role), attach its own
    /// MCID to the nearest enclosing element, then descend.
    fn walk(&mut self, frag: &Fragment, page: usize, parent: Option<usize>, mcid: &mut i32) {
        let holder = self.open_element(frag, page, parent);
        self.attach_content(frag, page, holder, mcid);
        for child in &frag.children {
            self.walk(child, page, holder, mcid);
        }
    }

    /// Create this fragment's struct element if it carries a role, returning the
    /// node to which its (and its descendants') content attaches.
    fn open_element(
        &mut self,
        frag: &Fragment,
        page: usize,
        parent: Option<usize>,
    ) -> Option<usize> {
        let role = match frag.role {
            Some(UaRole::Artifact) | None => return parent,
            Some(r) => r,
        };
        let idx = self.push_node(map_role(role), frag.alt.clone(), page);
        match parent {
            Some(p) => self.nodes[p].children.push(idx),
            None => self.roots.push(idx),
        }
        Some(idx)
    }

    /// Attach this fragment's own content MCID (if it paints real content in a
    /// content band) to its holder, conjuring a paragraph holder if there is none.
    fn attach_content(
        &mut self,
        frag: &Fragment,
        page: usize,
        holder: Option<usize>,
        mcid: &mut i32,
    ) {
        if !is_content(frag) {
            return;
        }
        let owner = holder.unwrap_or_else(|| self.synthetic_paragraph(page));
        let id = *mcid;
        *mcid += 1;
        self.nodes[owner].mcids.push(id);
    }

    /// A fresh top-level `P` element to own stray content (a split block whose
    /// wrapper landed on another page).
    fn synthetic_paragraph(&mut self, page: usize) -> usize {
        let idx = self.push_node(StructRole::P, None, page);
        self.roots.push(idx);
        idx
    }

    /// Push a new struct node, returning its index.
    fn push_node(&mut self, role: StructRole, alt: Option<String>, page: usize) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(StructNode {
            role,
            alt,
            page,
            children: Vec::new(),
            mcids: Vec::new(),
        });
        idx
    }
}

/// Whether a fragment contributes a content MCID in a content band (matches the
/// painter's [`wrap_kind`] `Content` arm).
fn is_content(frag: &Fragment) -> bool {
    matches!(
        frag.content,
        FragmentContent::TextLine { .. } | FragmentContent::Image(_)
    )
}

/// Map a layout [`UaRole`] to a `pdf_writer` structure role.
fn map_role(role: UaRole) -> StructRole {
    match role {
        UaRole::Group => StructRole::Div,
        UaRole::Heading(level) => heading_role(level),
        UaRole::Paragraph => StructRole::P,
        UaRole::List => StructRole::L,
        UaRole::ListItem => StructRole::LI,
        UaRole::ListBody => StructRole::LBody,
        UaRole::Table => StructRole::Table,
        UaRole::TableRow => StructRole::TR,
        UaRole::TableHeader => StructRole::TH,
        UaRole::TableData => StructRole::TD,
        UaRole::Figure => StructRole::Figure,
        UaRole::Span => StructRole::Span,
        // Artifacts never reach the tree (filtered in `open_element`).
        UaRole::Artifact => StructRole::NonStruct,
    }
}

/// The `H1`..`H6` role for a 1-based heading level (clamped to the valid range).
fn heading_role(level: u8) -> StructRole {
    match level.clamp(1, 6) {
        1 => StructRole::H1,
        2 => StructRole::H2,
        3 => StructRole::H3,
        4 => StructRole::H4,
        5 => StructRole::H5,
        _ => StructRole::H6,
    }
}

// --------------------------------------------------------------------------
// object writing
// --------------------------------------------------------------------------

impl UaPlan {
    /// Write every object the tagged-PDF plan owns: the struct-tree root, the
    /// `Document` element, every struct element, the per-page parent trees and
    /// the XMP metadata stream.
    pub(super) fn write(&self, pdf: &mut Pdf, page_refs: &[(Ref, Ref)], opts: &EmitOptions) {
        self.write_root(pdf);
        self.write_document(pdf, page_refs);
        self.write_nodes(pdf, page_refs);
        self.write_parent_trees(pdf);
        self.write_metadata(pdf, opts);
    }

    /// The `StructTreeRoot`: one `Document` child plus the parent-tree map. The
    /// parent tree is a single leaf number tree whose `/Nums` maps each page's
    /// `StructParents` key to that page's MCID→element array object.
    fn write_root(&self, pdf: &mut Pdf) {
        let mut root = pdf
            .indirect(self.root_ref)
            .start::<pdf_writer::writers::StructTreeRoot>();
        root.child(self.doc_ref);
        let mut pt = root.parent_tree();
        let mut nums = pt.nums();
        for (page, arr_ref) in self.parent_tree_refs.iter().enumerate() {
            nums.insert(page as i32, *arr_ref);
        }
        nums.finish();
        pt.finish();
        root.parent_tree_next_key(self.parent_tree_refs.len() as i32);
    }

    /// The `Document` element: every top-level page element is its child.
    fn write_document(&self, pdf: &mut Pdf, page_refs: &[(Ref, Ref)]) {
        let mut doc = pdf.struct_element(self.doc_ref);
        doc.kind(StructRole::Document);
        doc.parent(self.root_ref);
        let mut kids = doc.children();
        for &i in &self.roots {
            kids.struct_element(self.node_refs[i]);
        }
        kids.finish();
        doc.finish();
        let _ = page_refs;
    }

    /// Write one `StructElem` object per node.
    fn write_nodes(&self, pdf: &mut Pdf, page_refs: &[(Ref, Ref)]) {
        for i in 0..self.nodes.len() {
            self.write_node(pdf, i, page_refs);
        }
    }

    /// Write one struct element: its role, parent, page, optional alt, then its
    /// children (sub-elements followed by its own marked content ids).
    fn write_node(&self, pdf: &mut Pdf, i: usize, page_refs: &[(Ref, Ref)]) {
        let node = &self.nodes[i];
        let mut el = pdf.struct_element(self.node_refs[i]);
        el.kind(node.role);
        el.parent(self.parent_ref(i));
        el.page(page_refs[node.page].0);
        if let Some(alt) = &node.alt {
            el.alt(TextStr(alt));
        }
        write_scope(&mut el, node.role);
        write_kids(&mut el, node, &self.node_refs);
    }

    /// The parent object of node `i`: its enclosing element, or the `Document`.
    fn parent_ref(&self, i: usize) -> Ref {
        for (p, node) in self.nodes.iter().enumerate() {
            if node.children.contains(&i) {
                return self.node_refs[p];
            }
        }
        self.doc_ref
    }

    /// Write one parent-tree array object per page: MCID `k` on that page maps to
    /// the element that owns it (an array indexed by MCID).
    fn write_parent_trees(&self, pdf: &mut Pdf) {
        for (page, &arr_ref) in self.parent_tree_refs.iter().enumerate() {
            self.write_page_parent_tree(pdf, page, arr_ref);
        }
    }

    /// The MCID→element array for one page.
    fn write_page_parent_tree(&self, pdf: &mut Pdf, page: usize, arr_ref: Ref) {
        let owners = self.mcid_owners(page);
        let mut arr = pdf.indirect(arr_ref).array();
        for owner in owners {
            arr.item(owner);
        }
        arr.finish();
    }

    /// For a page, the owning element ref of each MCID `0..count`, in order.
    fn mcid_owners(&self, page: usize) -> Vec<Ref> {
        let count = self.mcid_counts[page] as usize;
        let mut owners = vec![self.doc_ref; count];
        for (i, node) in self.nodes.iter().enumerate() {
            if node.page == page {
                for &mcid in &node.mcids {
                    owners[mcid as usize] = self.node_refs[i];
                }
            }
        }
        owners
    }

    /// Write the XMP metadata packet (PDF/UA identification + `dc:title`).
    fn write_metadata(&self, pdf: &mut Pdf, opts: &EmitOptions) {
        let xmp = xmp_packet(opts.title.as_deref());
        pdf.metadata(self.metadata_ref, xmp.as_bytes()).finish();
    }
}

/// Give a `TH` cell a `/Scope /Column` table attribute so a screen reader can
/// associate it with its column (ISO 14289-1 §7.5). Column scope is the right
/// default for the single header row this engine produces (`<th>` in the first
/// `<tr>`); other roles get no attribute.
fn write_scope(el: &mut pdf_writer::writers::StructElement, role: StructRole) {
    use pdf_writer::types::TableHeaderScope;
    if role != StructRole::TH {
        return;
    }
    let mut attrs = el.attributes();
    attrs.push().table().scope(TableHeaderScope::Column);
    attrs.finish();
}

/// Write a struct element's `/K` children: nested elements then own MCIDs.
fn write_kids(el: &mut pdf_writer::writers::StructElement, node: &StructNode, refs: &[Ref]) {
    let mut kids = el.children();
    for &c in &node.children {
        kids.struct_element(refs[c]);
    }
    for &mcid in &node.mcids {
        kids.marked_content_id(mcid);
    }
    kids.finish();
}

/// Build the XMP metadata packet identifying the document as PDF/UA-1 and
/// carrying the title (required so `DisplayDocTitle` has something to show).
fn xmp_packet(title: Option<&str>) -> String {
    let title = title.unwrap_or("Untitled");
    let title = xml_escape(title);
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\
<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{title}</rdf:li></rdf:Alt></dc:title>\
</rdf:Description>\
<rdf:Description rdf:about=\"\" xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\">\
<pdfuaid:part>1</pdfuaid:part>\
</rdf:Description>\
</rdf:RDF></x:xmpmeta>\
<?xpacket end=\"r\"?>"
    )
}

/// Minimal XML text escaping for the title in the XMP packet.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
