use crate::html5::node::arena::NodeArena;
use crate::html5::node::data::doctype::DocTypeData;
use crate::html5::node::data::{comment::CommentData, text::TextData};
use crate::html5::node::HTML_NAMESPACE;
use crate::html5::node::{Node, NodeData, NodeId};
use crate::html5::parser::quirks::QuirksMode;
use crate::html5::parser::tree_builder::TreeBuilder;
use crate::types::{Error, Result};
use alloc::rc::Rc;
use core::fmt;
use core::fmt::Debug;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Display;
use std::ops::{Deref, DerefMut};

/// Type of the given document
#[derive(PartialEq, Debug, Copy, Clone)]
pub enum DocumentType {
    /// HTML document
    HTML,
    /// Iframe source document
    IframeSrcDoc,
}

/// Defines a document fragment which can be attached to for instance a <template> element
#[derive(PartialEq)]
pub struct DocumentFragment {
    /// Node elements inside this fragment
    arena: NodeArena,
    /// Document handle of the parent
    pub doc: DocumentHandle,
    /// Host node on which this fragment is attached
    host: NodeId,
}

impl Clone for DocumentFragment {
    /// Clones the document fragment
    fn clone(&self) -> Self {
        Self {
            arena: self.arena.clone(),
            doc: Document::clone(&self.doc),
            host: self.host,
        }
    }
}

impl Debug for DocumentFragment {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DocumentFragment")
    }
}

impl DocumentFragment {
    /// Creates a new document fragment and attaches it to "host" node inside "doc"
    pub(crate) fn new(doc: DocumentHandle, host: NodeId) -> Self {
        Self {
            arena: NodeArena::new(),
            doc,
            host,
        }
    }
}

/// Enum of tasks that can be performed to add or update
/// update nodes in the tree.
///
/// These tasks are generated by a TreeBuilder which is implemented
/// by DocumentTaskQueue which holds a handle to the actual Document
/// to commit changes to.
pub enum DocumentTask {
    CreateElement {
        name: String,
        parent_id: NodeId,
        position: Option<usize>,
        namespace: String,
    },
    CreateText {
        content: String,
        parent_id: NodeId,
    },
    CreateComment {
        content: String,
        parent_id: NodeId,
    },
    InsertAttribute {
        key: String,
        value: String,
        element_id: NodeId,
    },
}

/// Queue of tasks that will mutate the document to add/update
/// nodes in the tree. These tasks are performed sequentially in the
/// order they are created.
///
/// Once tasks are queued up, a call to flush() will commit all changes
/// to the DOM. If there are errors during the application of these changes,
/// flush() will return a list of the errors encountered but execution is not halted.
///
/// create_element() will generate and return a new NodeId for the parser to keep
/// track of the current context node and optionally store this in a list of open elements.
/// When encountering a closing tag, the parser must pop this ID off of its list.
pub struct DocumentTaskQueue {
    /// Internal counter of the next ID to generate from the NodeArena
    /// without actually registering the node.
    /// WARNING: if nodes are registered in the arena while tasks are being queued
    /// this could lead to conflicts in NodeIds. NodeArena should NOT be used directly
    /// if using a DocumentTaskQueue.
    next_node_id: NodeId,
    /// Reference to the document to commit changes to
    pub(crate) document: DocumentHandle,
    /// List of tasks to commit upon flush() which is cleared after execution finishes.
    // IMPLEMENTATION NOTE: using a vec here since I'm assuming we are
    // executing all tasks at once. If we need to support stopping task
    // execution midway, then maybe a real "queue" structure that pops
    // completed tasks is needed.
    pub(crate) tasks: Vec<DocumentTask>,
}

impl DocumentTaskQueue {
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    fn flush(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        for current_task in &self.tasks {
            match current_task {
                DocumentTask::CreateElement {
                    name,
                    parent_id,
                    position,
                    namespace,
                } => {
                    self.document
                        .create_element(name, *parent_id, *position, namespace);
                }
                DocumentTask::CreateText { content, parent_id } => {
                    self.document.create_text(content, *parent_id);
                }
                DocumentTask::CreateComment { content, parent_id } => {
                    self.document.create_comment(content, *parent_id);
                }
                DocumentTask::InsertAttribute {
                    key,
                    value,
                    element_id,
                } => {
                    if let Err(err) = self.document.insert_attribute(key, value, *element_id) {
                        errors.push(err.to_string());
                    }
                }
            }
        }
        self.tasks.clear();

        errors
    }
}

// See tree_builder.rs for method comments
impl TreeBuilder for DocumentTaskQueue {
    fn create_element(
        &mut self,
        name: &str,
        parent_id: NodeId,
        position: Option<usize>,
        namespace: &str,
    ) -> NodeId {
        let element = DocumentTask::CreateElement {
            name: name.to_owned(),
            parent_id,
            position,
            namespace: namespace.to_owned(),
        };
        let new_id = self.next_node_id;
        self.next_node_id = self.next_node_id.next();
        self.tasks.push(element);

        new_id
    }

    fn create_text(&mut self, content: &str, parent_id: NodeId) {
        let text = DocumentTask::CreateText {
            content: content.to_owned(),
            parent_id,
        };
        self.tasks.push(text);
    }

    fn create_comment(&mut self, content: &str, parent_id: NodeId) {
        let comment = DocumentTask::CreateComment {
            content: content.to_owned(),
            parent_id,
        };
        self.tasks.push(comment);
    }

    fn insert_attribute(&mut self, key: &str, value: &str, element_id: NodeId) -> Result<()> {
        let attribute = DocumentTask::InsertAttribute {
            key: key.to_owned(),
            value: value.to_owned(),
            element_id,
        };
        self.tasks.push(attribute);
        Ok(())
    }
}

impl DocumentTaskQueue {
    pub fn new(document: &DocumentHandle) -> Self {
        let document = Document::clone(document);
        let next_node_id = document.get().arena.peek_next_id();
        Self {
            next_node_id,
            document,
            tasks: Vec::new(),
        }
    }
}

/// Defines a document
#[derive(Debug, PartialEq)]
pub struct Document {
    /// Holds and owns all nodes in the document
    pub(crate) arena: NodeArena,
    /// HTML elements with ID (e.g., <div id="myid">)
    named_id_elements: HashMap<String, NodeId>,
    /// Document type of this document
    pub doctype: DocumentType,
    /// Quirks mode of this document
    pub quirks_mode: QuirksMode,
}

impl Default for Document {
    /// Returns a default document
    fn default() -> Self {
        Self {
            arena: NodeArena::new(),
            named_id_elements: HashMap::new(),
            doctype: DocumentType::HTML,
            quirks_mode: QuirksMode::NoQuirks,
        }
    }
}

impl Document {
    /// Creates a new document
    pub fn new() -> Self {
        let arena = NodeArena::new();
        Self {
            arena,
            named_id_elements: HashMap::new(),
            doctype: DocumentType::HTML,
            quirks_mode: QuirksMode::NoQuirks,
        }
    }

    /// Returns a shared reference-counted handle for the document
    pub fn shared() -> DocumentHandle {
        DocumentHandle(Rc::new(RefCell::new(Self::new())))
    }

    /// Fast clone of a lightweight reference-counted handle for the document.  This is a shallow
    /// clone, and different handles will see the same underlying document.
    pub fn clone(handle: &DocumentHandle) -> DocumentHandle {
        DocumentHandle(Rc::clone(&handle.0))
    }

    pub(crate) fn print_nodes(&self) {
        self.arena.print_nodes();
    }

    /// Fetches a node by id or returns None when no node with this ID is found
    pub fn get_node_by_id(&self, node_id: NodeId) -> Option<&Node> {
        self.arena.get_node(node_id)
    }

    /// Fetches a mutable node by id or returns None when no node with this ID is found
    pub fn get_node_by_id_mut(&mut self, node_id: NodeId) -> Option<&mut Node> {
        self.arena.get_node_mut(node_id)
    }

    /// Fetches a node by named id (string) or returns None when no node with this ID is found
    pub fn get_node_by_named_id(&self, named_id: &str) -> Option<&Node> {
        let node_id = self.named_id_elements.get(named_id)?;
        self.arena.get_node(*node_id)
    }

    /// Fetches a mutable node by named id (string) or returns None when no node with this ID is found
    pub fn get_node_by_named_id_mut(&mut self, named_id: &str) -> Option<&mut Node> {
        let node_id = self.named_id_elements.get(named_id)?;
        self.arena.get_node_mut(*node_id)
    }

    /// according to HTML5 spec: 3.2.3.1
    /// https://www.w3.org/TR/2011/WD-html5-20110405/elements.html#the-id-attribute
    fn validate_id_attribute_value(&self, value: &str) -> bool {
        if value.contains(char::is_whitespace) {
            return false;
        }

        if value.is_empty() {
            return false;
        }

        // must contain at least one character,
        // but doesn't specify it should *start* with a character
        value.contains(char::is_alphabetic)
    }

    pub fn add_new_node(&mut self, node: Node) -> NodeId {
        // if a node contains attributes when adding to the tree,
        // be sure to handle the special attributes "id" and "class"
        // which need to by queryable by the DOM
        let mut node_named_id: Option<String> = None;
        if let NodeData::Element(element) = &node.data {
            if let Some(named_id) = element.attributes.get("id") {
                node_named_id = Some(named_id.clone());
            }
        }

        // Register the node if needed
        let node_id = if !node.is_registered {
            self.arena.register_node(node)
        } else {
            node.id
        };

        // update the node's ID (it uses default ID when first created)
        if let Some(node) = self.get_node_by_id_mut(node_id) {
            if let NodeData::Element(element) = &mut node.data {
                element.set_id(node_id);
            }
        }

        // make named_id (if present) queryable in DOM if it's not mapped already
        if let Some(node_named_id) = node_named_id {
            if !self.named_id_elements.contains_key(&node_named_id)
                && self.validate_id_attribute_value(&node_named_id)
            {
                self.named_id_elements
                    .insert(node_named_id.to_owned(), node_id);
            }
        }

        node_id
    }

    /// Inserts a node to the parent node at the given position in the children (or none
    /// to add at the end). Will automatically register the node if not done so already
    pub fn add_node(&mut self, node: Node, parent_id: NodeId, position: Option<usize>) -> NodeId {
        let node_id = self.add_new_node(node);

        self.attach_node_to_parent(node_id, parent_id, position);

        node_id
    }

    /// Relocates a node to another parent node
    pub fn relocate(&mut self, node_id: NodeId, parent_id: NodeId) {
        let node = self.arena.get_node_mut(node_id).unwrap();
        if !node.is_registered {
            panic!("Node is not registered to the arena");
        }

        if node.parent.is_some() && node.parent.unwrap() == parent_id {
            // Nothing to do when we want to relocate to its own parent
            return;
        }

        self.detach_node_from_parent(node_id);
        self.attach_node_to_parent(node_id, parent_id, None);
    }

    /// Adds the node as a child the parent node. If position is given, it will be inserted as a
    /// child at that given position
    pub fn attach_node_to_parent(
        &mut self,
        node_id: NodeId,
        parent_id: NodeId,
        position: Option<usize>,
    ) -> bool {
        //check if any children of node have parent as child
        if parent_id == node_id || self.has_cyclic_reference(node_id, parent_id) {
            return false;
        }

        if let Some(parent_node) = self.get_node_by_id_mut(parent_id) {
            // Make sure position can never be larger than the number of children in the parent
            if let Some(mut position) = position {
                if position > parent_node.children.len() {
                    position = parent_node.children.len();
                }
                parent_node.children.insert(position, node_id);
            } else {
                // No position given, add to end of the children list
                parent_node.children.push(node_id);
            }
        }

        let node = self.arena.get_node_mut(node_id).unwrap();
        node.parent = Some(parent_id);

        true
    }

    /// Separates the given node from its parent node (if any)
    pub fn detach_node_from_parent(&mut self, node_id: NodeId) {
        let parent = self.get_node_by_id(node_id).expect("node not found").parent;

        if let Some(parent_id) = parent {
            let parent_node = self
                .get_node_by_id_mut(parent_id)
                .expect("parent node not found");
            parent_node.children.retain(|&id| id != node_id);

            let node = self.get_node_by_id_mut(node_id).expect("node not found");
            node.parent = None;
        }
    }

    /// returns the root node
    pub fn get_root(&self) -> &Node {
        self.arena
            .get_node(NodeId::root())
            .expect("Root node not found !?")
    }

    /// Returns true when the given parent_id is a child of the node_id
    pub fn has_cyclic_reference(&self, node_id: NodeId, parent_id: NodeId) -> bool {
        has_child_recursive(&self.arena, node_id, parent_id)
    }
}

/// Returns true when the parent node has the child node as a child, or if any of the children of
/// the parent node have the child node as a child.
fn has_child_recursive(arena: &NodeArena, parent_id: NodeId, child_id: NodeId) -> bool {
    let node = arena.get_node(parent_id).cloned();
    if node.is_none() {
        return false;
    }

    let node = node.unwrap();
    for id in node.children.iter() {
        if *id == child_id {
            return true;
        }
        let child = arena.get_node(*id).cloned();
        if has_child(arena, child, child_id) {
            return true;
        }
    }
    false
}

fn has_child(arena: &NodeArena, parent: Option<Node>, child_id: NodeId) -> bool {
    let parent_node = if let Some(node) = parent {
        node
    } else {
        return false;
    };

    if parent_node.children.is_empty() {
        return false;
    }

    for id in parent_node.children {
        if id == child_id {
            return true;
        }
        let node = arena.get_node(id).cloned();
        if has_child(arena, node, child_id) {
            return true;
        }
    }

    false
}

impl Document {
    /// Print a node and all its children in a tree-like structure
    pub fn print_tree(&self, node: &Node, prefix: String, last: bool, f: &mut fmt::Formatter) {
        let mut buffer = prefix.clone();
        if last {
            buffer.push_str("└─ ");
        } else {
            buffer.push_str("├─ ");
        }

        // buffer.push_str(&format!("({:?}) ", node.id.as_usize()));

        match &node.data {
            NodeData::Document(_) => {
                _ = writeln!(f, "{}Document", buffer);
            }
            NodeData::DocType(DocTypeData {
                name,
                pub_identifier,
                sys_identifier,
            }) => {
                _ = writeln!(
                    f,
                    r#"{buffer}<!DOCTYPE {name} "{pub_identifier}" "{sys_identifier}">"#,
                );
            }
            NodeData::Text(TextData { value, .. }) => {
                _ = writeln!(f, "{}\"{}\"", buffer, value);
            }
            NodeData::Comment(CommentData { value, .. }) => {
                _ = writeln!(f, "{}<!-- {} -->", buffer, value);
            }
            NodeData::Element(element) => {
                _ = write!(f, "{}<{}", buffer, element.name);
                for (key, value) in element.attributes.iter() {
                    _ = write!(f, " {}={}", key, value);
                }
                _ = writeln!(f, ">");
            }
        }

        if prefix.len() > 40 {
            _ = writeln!(f, "...");
            return;
        }

        let mut buffer = prefix;
        if last {
            buffer.push_str("   ");
        } else {
            buffer.push_str("│  ");
        }

        let len = node.children.len();
        for (i, child) in node.children.iter().enumerate() {
            let child = self.arena.get_node(*child).expect("Child not found");
            self.print_tree(child, buffer.clone(), i == len - 1, f);
        }
    }
}

impl Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.print_tree(self.get_root(), "".to_string(), true, f);
        Ok(())
    }
}

#[derive(Debug)]
pub struct DocumentHandle(Rc<RefCell<Document>>);

impl Display for DocumentHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.borrow())
    }
}

impl PartialEq for DocumentHandle {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

// NOTE: it is preferred to use Document::clone() when
// copying a DocumentHandle reference. However, for
// any structs using this handle that use #[derive(Clone)],
// this implementation is required.
impl Clone for DocumentHandle {
    fn clone(&self) -> DocumentHandle {
        DocumentHandle(Rc::clone(&self.0))
    }
}

impl Eq for DocumentHandle {}

impl DocumentHandle {
    /// Retrieves a immutable reference to the document
    pub fn get(&self) -> impl Deref<Target = Document> + '_ {
        self.0.borrow()
    }

    /// Retrieves a mutable reference to the document
    pub fn get_mut(&mut self) -> impl DerefMut<Target = Document> + '_ {
        self.0.borrow_mut()
    }

    /// Attaches a node to the parent node at the given position in the children (or none
    /// to add at the end).
    pub fn attach_node_to_parent(
        &mut self,
        node_id: NodeId,
        parent_id: NodeId,
        position: Option<usize>,
    ) -> bool {
        self.get_mut()
            .attach_node_to_parent(node_id, parent_id, position)
    }

    /// Separates the given node from its parent node (if any)
    pub fn detach_node_from_parent(&mut self, node_id: NodeId) {
        self.get_mut().detach_node_from_parent(node_id)
    }

    /// Inserts a node to the parent node at the given position in the children (or none
    /// to add at the end). Will automatically register the node if not done so already
    /// Returns the node ID of the inserted node
    pub fn add_node(&mut self, node: Node, parent_id: NodeId, position: Option<usize>) -> NodeId {
        self.get_mut().add_node(node, parent_id, position)
    }

    /// Relocates a node to another parent node
    pub fn relocate(&mut self, node_id: NodeId, parent_id: NodeId) {
        self.get_mut().relocate(node_id, parent_id)
    }

    /// Returns true when there is a cyclic reference from the given node_id to the parent_id
    pub fn has_cyclic_reference(&self, node_id: NodeId, parent_id: NodeId) -> bool {
        self.get().has_cyclic_reference(node_id, parent_id)
    }
}

impl TreeBuilder for DocumentHandle {
    /// Creates and attaches a new element node to the document
    fn create_element(
        &mut self,
        name: &str,
        parent_id: NodeId,
        position: Option<usize>,
        namespace: &str,
    ) -> NodeId {
        let new_element = Node::new_element(self, name, HashMap::new(), namespace);
        self.add_node(new_element, parent_id, position)
    }

    /// Creates and attaches a new text node to the document
    fn create_text(&mut self, content: &str, parent_id: NodeId) {
        let new_text = Node::new_text(self, content);
        self.add_node(new_text, parent_id, None);
    }

    /// Creates and attaches a new comment node to the document
    fn create_comment(&mut self, content: &str, parent_id: NodeId) {
        let new_comment = Node::new_comment(self, content);
        self.add_node(new_comment, parent_id, None);
    }

    /// Inserts an attribute to an element node.
    /// If node is not an element or if passing an invalid attribute value, returns an Err()
    fn insert_attribute(&mut self, key: &str, value: &str, element_id: NodeId) -> Result<()> {
        if !self.get().validate_id_attribute_value(value) {
            return Err(Error::DocumentTask(format!(
                "Attribute value '{}' did not pass validation",
                value
            )));
        }

        if let Some(node) = self.get_mut().get_node_by_id_mut(element_id) {
            if let NodeData::Element(element) = &mut node.data {
                element.attributes.insert(key.to_owned(), value.to_owned());
            } else {
                return Err(Error::DocumentTask(format!(
                    "Node ID {} is not an element",
                    element_id
                )));
            }
        } else {
            return Err(Error::DocumentTask(format!(
                "Node ID {} not found",
                element_id
            )));
        }

        // special cases that need to sync with DOM
        match key {
            "id" => {
                // if ID is already in use, ignore
                if !self.get().named_id_elements.contains_key(value) {
                    self.get_mut()
                        .named_id_elements
                        .insert(value.to_owned(), element_id);
                }
            }
            "class" => {
                // this will be upcoming in a later PR
                todo!()
            }
            _ => {}
        }
        Ok(())
    }
}

/// This struct will be used to create a fully initialized document or document fragment
pub struct DocumentBuilder;

impl DocumentBuilder {
    /// Creates a new document with a document root node
    pub fn new_document() -> DocumentHandle {
        let mut doc = Document::shared();

        let handle = &Document::clone(&doc);
        let node = Node::new_document(handle);
        doc.get_mut().arena.register_node(node);

        doc
    }

    /// Creates a new document fragment with the context as the root node
    pub fn new_document_fragment(context: Node) -> DocumentHandle {
        let mut doc = Document::shared();
        doc.get_mut().doctype = DocumentType::HTML;

        if context.document.get().quirks_mode == QuirksMode::Quirks {
            doc.get_mut().quirks_mode = QuirksMode::Quirks;
        } else if context.document.get().quirks_mode == QuirksMode::LimitedQuirks {
            doc.get_mut().quirks_mode = QuirksMode::LimitedQuirks;
        }

        // @TODO: Set tokenizer state based on context element

        let html_node = Node::new_element(&doc, "html", HashMap::new(), HTML_NAMESPACE);
        // doc.get_mut().arena.register_node(html_node);
        doc.add_node(html_node, NodeId::root(), None);

        doc
    }
}

#[cfg(test)]
mod tests {
    use crate::html5::node::{NodeTrait, NodeType, HTML_NAMESPACE};
    use crate::html5::parser::document::{DocumentBuilder, DocumentTaskQueue};
    use crate::html5::parser::tree_builder::TreeBuilder;
    use crate::html5::parser::{Node, NodeData, NodeId};
    use std::collections::HashMap;

    #[test]
    fn relocate() {
        let mut document = DocumentBuilder::new_document();

        let parent = Node::new_element(&document, "parent", HashMap::new(), HTML_NAMESPACE);
        let node1 = Node::new_element(&document, "div1", HashMap::new(), HTML_NAMESPACE);
        let node2 = Node::new_element(&document, "div2", HashMap::new(), HTML_NAMESPACE);
        let node3 = Node::new_element(&document, "div3", HashMap::new(), HTML_NAMESPACE);
        let node3_1 = Node::new_element(&document, "div3_1", HashMap::new(), HTML_NAMESPACE);

        let parent_id = document.get_mut().add_node(parent, NodeId::from(0), None);
        let node1_id = document.get_mut().add_node(node1, parent_id, None);
        let node2_id = document.get_mut().add_node(node2, parent_id, None);
        let node3_id = document.get_mut().add_node(node3, parent_id, None);
        let node3_1_id = document.get_mut().add_node(node3_1, node3_id, None);

        assert_eq!(
            format!("{}", document),
            r#"└─ Document
   └─ <parent>
      ├─ <div1>
      ├─ <div2>
      └─ <div3>
         └─ <div3_1>
"#
        );

        document.get_mut().relocate(node3_1_id, node1_id);
        assert_eq!(
            format!("{}", document),
            r#"└─ Document
   └─ <parent>
      ├─ <div1>
      │  └─ <div3_1>
      ├─ <div2>
      └─ <div3>
"#
        );

        document.get_mut().relocate(node1_id, node2_id);
        assert_eq!(
            format!("{}", document),
            r#"└─ Document
   └─ <parent>
      ├─ <div2>
      │  └─ <div1>
      │     └─ <div3_1>
      └─ <div3>
"#
        );
    }

    #[test]
    fn duplicate_named_id_elements() {
        let mut document = DocumentBuilder::new_document();

        let div_1 = document.create_element("div", NodeId::root(), None, HTML_NAMESPACE);
        let div_2 = document.create_element("div", NodeId::root(), None, HTML_NAMESPACE);

        // when adding duplicate IDs, our current implementation will ignore duplicates.
        let mut res = document.insert_attribute("id", "myid", div_1);
        assert!(res.is_ok());
        res = document.insert_attribute("id", "myid", div_2);
        assert!(res.is_ok());

        assert_eq!(
            document.get().get_node_by_named_id("myid").unwrap().id,
            div_1
        );
    }

    #[test]
    fn verify_node_ids_in_element_data() {
        let mut document = DocumentBuilder::new_document();

        let node1 = Node::new_element(&document, "div", HashMap::new(), HTML_NAMESPACE);
        let node2 = Node::new_element(&document, "div", HashMap::new(), HTML_NAMESPACE);

        document.get_mut().add_node(node1, NodeId::from(0), None);
        document.get_mut().add_node(node2, NodeId::from(0), None);

        let doc_ptr = document.get();

        let get_node1 = doc_ptr.get_node_by_id(NodeId::from(1)).unwrap();
        let get_node2 = doc_ptr.get_node_by_id(NodeId::from(2)).unwrap();

        let NodeData::Element(element1) = &get_node1.data else {
            panic!()
        };

        assert_eq!(element1.node_id, NodeId::from(1));

        let NodeData::Element(element2) = &get_node2.data else {
            panic!()
        };

        assert_eq!(element2.node_id, NodeId::from(2));
    }

    #[test]
    fn document_task_queue() {
        let document = DocumentBuilder::new_document();

        // Using task queue to create the following structure initially:
        // <div>
        //   <p>
        //     <!-- comment inside p -->
        //     hey
        //   </p>
        //   <!-- comment inside div -->
        // </div>

        // then flush the queue and use it again to add an attribute to <p>:
        // <p id="myid">hey</p>
        let mut task_queue = DocumentTaskQueue::new(&document);

        // NOTE: only elements return the ID
        let div_id = task_queue.create_element("div", NodeId::root(), None, HTML_NAMESPACE);
        assert_eq!(div_id, NodeId::from(1));

        let p_id = task_queue.create_element("p", div_id, None, HTML_NAMESPACE);
        assert_eq!(p_id, NodeId::from(2));

        task_queue.create_comment("comment inside p", p_id);
        task_queue.create_text("hey", p_id);
        task_queue.create_comment("comment inside div", div_id);

        // at this point, the DOM should have NO nodes (besides root)
        assert_eq!(document.get().arena.count_nodes(), 1);

        // validate our queue is loaded
        assert!(!task_queue.is_empty());
        let errors = task_queue.flush();
        assert!(errors.is_empty());

        // validate queue is empty
        assert!(task_queue.is_empty());

        // DOM should now have all our nodes
        assert_eq!(document.get().arena.count_nodes(), 6);

        // NOTE: these checks are scoped separately since this is using an
        // immutable borrow and we make a mutable borrow after (to insert the attribute).
        // We need this immutable borrow to die off before making a new mutable borrow
        // (and again an immutable borrow for validation afterwards)
        {
            // validate DOM is correctly laid out
            let doc_read = document.get();
            let root = doc_read.get_root(); // <!DOCTYPE html>
            let root_children = &root.children;

            // div child
            let div_child = doc_read.get_node_by_id(root_children[0]).unwrap();
            assert_eq!(div_child.type_of(), NodeType::Element);
            assert_eq!(div_child.name, "div");
            let div_children = &div_child.children;

            // p child
            let p_child = doc_read.get_node_by_id(div_children[0]).unwrap();
            assert_eq!(p_child.type_of(), NodeType::Element);
            assert_eq!(p_child.name, "p");
            let p_children = &p_child.children;

            // comment inside p
            let p_comment = doc_read.get_node_by_id(p_children[0]).unwrap();
            assert_eq!(p_comment.type_of(), NodeType::Comment);
            let NodeData::Comment(p_comment_data) = &p_comment.data else {
                panic!()
            };
            assert_eq!(p_comment_data.value, "comment inside p");

            // body inside p
            let p_body = doc_read.get_node_by_id(p_children[1]).unwrap();
            assert_eq!(p_body.type_of(), NodeType::Text);
            let NodeData::Text(p_body_data) = &p_body.data else {
                panic!()
            };
            assert_eq!(p_body_data.value, "hey");

            // comment inside div
            let div_comment = doc_read.get_node_by_id(div_children[1]).unwrap();
            assert_eq!(div_comment.type_of(), NodeType::Comment);
            let NodeData::Comment(div_comment_data) = &div_comment.data else {
                panic!()
            };
            assert_eq!(div_comment_data.value, "comment inside div");
        }

        // use task queue again to add an ID attribute
        // NOTE: inserting attribute in task queue always succeeds
        // since it doesn't touch DOM until flush
        let _ = task_queue.insert_attribute("id", "myid", p_id);
        let errors = task_queue.flush();
        assert!(errors.is_empty());

        let doc_read = document.get();
        // validate ID is searchable in dom
        assert_eq!(*doc_read.named_id_elements.get("myid").unwrap(), p_id);

        // validate attribute is applied to underlying element
        let p_node = doc_read.get_node_by_id(p_id).unwrap();
        let NodeData::Element(p_element) = &p_node.data else {
            panic!()
        };
        assert_eq!(p_element.attributes.get("id").unwrap(), "myid");
    }

    #[test]
    fn task_queue_insert_attribute_failues() {
        let document = DocumentBuilder::new_document();

        let mut task_queue = DocumentTaskQueue::new(&document);
        let div_id = task_queue.create_element("div", NodeId::root(), None, HTML_NAMESPACE);
        task_queue.create_comment("content", div_id); // this is NodeId::from(2)
        task_queue.flush();

        // NOTE: inserting attribute in task queue always succeeds
        // since it doesn't touch DOM until flush
        let _ = task_queue.insert_attribute("id", "myid", NodeId::from(2));
        let _ = task_queue.insert_attribute("id", "myid", NodeId::from(42));
        let _ = task_queue.insert_attribute("id", "my id", NodeId::from(1));
        let _ = task_queue.insert_attribute("id", "123", NodeId::from(1));
        let _ = task_queue.insert_attribute("id", "", NodeId::from(1));
        let errors = task_queue.flush();
        assert_eq!(errors.len(), 5);
        assert_eq!(
            errors[0],
            "document task error: Node ID 2 is not an element",
        );
        assert_eq!(errors[1], "document task error: Node ID 42 not found");
        assert_eq!(
            errors[2],
            "document task error: Attribute value 'my id' did not pass validation",
        );
        assert_eq!(
            errors[3],
            "document task error: Attribute value '123' did not pass validation",
        );
        assert_eq!(
            errors[4],
            "document task error: Attribute value '' did not pass validation",
        );

        // validate that changes did not apply to DOM
        let doc_read = document.get();
        assert!(doc_read.named_id_elements.get("myid").is_none());
        assert!(doc_read.named_id_elements.get("my id").is_none());
        assert!(doc_read.named_id_elements.get("123").is_none());
        assert!(doc_read.named_id_elements.get("").is_none());
    }

    // this is basically a replica of document_task_queue() test
    // but using tree builder directly instead of the task queue
    #[test]
    fn document_tree_builder() {
        let mut document = DocumentBuilder::new_document();

        // Using tree builder to create the following structure:
        // <div>
        //   <p id="myid">
        //     <!-- comment inside p -->
        //     hey
        //   </p>
        //   <!-- comment inside div -->
        // </div>

        // NOTE: only elements return the ID
        let div_id = document.create_element("div", NodeId::root(), None, HTML_NAMESPACE);
        assert_eq!(div_id, NodeId::from(1));

        let p_id = document.create_element("p", div_id, None, HTML_NAMESPACE);
        assert_eq!(p_id, NodeId::from(2));

        document.create_comment("comment inside p", p_id);
        document.create_text("hey", p_id);
        document.create_comment("comment inside div", div_id);

        let res = document.insert_attribute("id", "myid", p_id);
        assert!(res.is_ok());

        // DOM should now have all our nodes
        assert_eq!(document.get().arena.count_nodes(), 6);

        // validate DOM is correctly laid out
        let doc_read = document.get();
        let root = doc_read.get_root(); // <!DOCTYPE html>
        let root_children = &root.children;

        // div child
        let div_child = doc_read.get_node_by_id(root_children[0]).unwrap();
        assert_eq!(div_child.type_of(), NodeType::Element);
        assert_eq!(div_child.name, "div");
        let div_children = &div_child.children;

        // p child
        let p_child = doc_read.get_node_by_id(div_children[0]).unwrap();
        assert_eq!(p_child.type_of(), NodeType::Element);
        assert_eq!(p_child.name, "p");
        let p_children = &p_child.children;

        // comment inside p
        let p_comment = doc_read.get_node_by_id(p_children[0]).unwrap();
        assert_eq!(p_comment.type_of(), NodeType::Comment);
        let NodeData::Comment(p_comment_data) = &p_comment.data else {
            panic!()
        };
        assert_eq!(p_comment_data.value, "comment inside p");

        // body inside p
        let p_body = doc_read.get_node_by_id(p_children[1]).unwrap();
        assert_eq!(p_body.type_of(), NodeType::Text);
        let NodeData::Text(p_body_data) = &p_body.data else {
            panic!()
        };
        assert_eq!(p_body_data.value, "hey");

        // comment inside div
        let div_comment = doc_read.get_node_by_id(div_children[1]).unwrap();
        assert_eq!(div_comment.type_of(), NodeType::Comment);
        let NodeData::Comment(div_comment_data) = &div_comment.data else {
            panic!()
        };
        assert_eq!(div_comment_data.value, "comment inside div");

        // validate ID is searchable in dom
        assert_eq!(*doc_read.named_id_elements.get("myid").unwrap(), p_id);

        // validate attribute is applied to underlying element
        let p_node = doc_read.get_node_by_id(p_id).unwrap();
        let NodeData::Element(p_element) = &p_node.data else {
            panic!()
        };
        assert_eq!(p_element.attributes.get("id").unwrap(), "myid");
    }
}
