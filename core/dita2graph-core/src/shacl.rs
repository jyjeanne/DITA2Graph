//! Build-time SHACL check of the RDF export (`docs/semantic-model.md`,
//! "Validation"): `rdf/graph.ttl` against `rdf/shapes.ttl`, the ontology
//! loaded beside the data, in a closed world -- what pySHACL runs in CI
//! with `-i none`, so the two must agree, result for result (the tests
//! below compare with pySHACL's own results on `tests/data/rdf-invalid.ttl`).
//!
//! It implements the SHACL the d2g shapes use, not all of SHACL: targets
//! (`sh:targetClass`, `sh:targetSubjectsOf`, `sh:targetObjectsOf`), property
//! shapes whose path is a predicate or `sh:inversePath` of one, the
//! components in [`Constraint`], and SHACL-SPARQL `sh:select` constraints on
//! node shapes, run on an in-memory Oxigraph store. A shape using anything
//! else is an error, never silently skipped: a constraint the build did not
//! check would otherwise read as one that passed.

use anyhow::{Context, Result, anyhow, bail};
use oxigraph::io::RdfFormat;
use oxigraph::model::vocab::{rdf, rdfs};
use oxigraph::model::{
    NamedNode, NamedNodeRef, NamedOrBlankNode, NamedOrBlankNodeRef, Term, TermRef,
};
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

const SH: &str = "http://www.w3.org/ns/shacl#";

/// `sh:resultSeverity`: a violation fails the build, the others are reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShaclSeverity {
    Violation,
    Warning,
    Info,
}

/// One validation result, as SHACL reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaclResult {
    pub severity: ShaclSeverity,
    /// The constraint component's local name (`MinCountConstraintComponent`).
    pub component: &'static str,
    /// The node shape the constraint belongs to (`d2g:NodeShape`...).
    pub shape: String,
    pub focus: Term,
    /// `<iri>` or `^<iri>` (an inverse path); none for a node constraint.
    pub path: Option<String>,
    pub value: Option<Term>,
    pub message: String,
}

impl ShaclResult {
    /// One line for a person: the shape, the focus node, the message, and
    /// the offending value when it is not the focus node itself.
    pub fn describe(&self) -> String {
        let value = match &self.value {
            Some(value) if *value != self.focus => format!(" (value {})", plain(value)),
            _ => String::new(),
        };
        format!(
            "{} on {}: {}{value}",
            self.shape,
            plain(&self.focus),
            self.message
        )
    }

    /// `severity|component|focus|path|value`, the form the tests compare
    /// with pySHACL's results.
    pub fn line(&self) -> String {
        format!(
            "{:?}|{}|{}|{}|{}",
            self.severity,
            self.component,
            plain(&self.focus),
            self.path.as_deref().unwrap_or("-"),
            self.value.as_ref().map(plain).unwrap_or_else(|| "-".into()),
        )
    }
}

/// What a check found.
#[derive(Debug, Default)]
pub struct ShaclReport {
    pub results: Vec<ShaclResult>,
    /// Focus nodes checked, over every shape.
    pub focus_nodes: usize,
}

impl ShaclReport {
    pub fn count(&self, severity: ShaclSeverity) -> usize {
        self.results
            .iter()
            .filter(|r| r.severity == severity)
            .count()
    }

    /// No violation: what the build requires. Warnings and notes pass.
    pub fn passes(&self) -> bool {
        self.count(ShaclSeverity::Violation) == 0
    }
}

/// Checks `rdf/graph.ttl` against `rdf/shapes.ttl`, with `rdf/ontology.ttl`
/// loaded beside the data, as `write_rdf` writes them.
pub fn validate_rdf_dir(rdf_dir: &Path) -> Result<ShaclReport> {
    let read = |name: &str| {
        let path = rdf_dir.join(name);
        fs::read(&path).with_context(|| format!("reading {}", path.display()))
    };
    validate(
        &read("graph.ttl")?,
        &read("ontology.ttl")?,
        &read("shapes.ttl")?,
    )
}

/// Checks a Turtle data graph against Turtle shapes, the ontology loaded
/// into the data graph (pySHACL's `ont_graph`, without inference).
pub fn validate(data: &[u8], ontology: &[u8], shapes: &[u8]) -> Result<ShaclReport> {
    let store = Store::new().context("creating the in-memory RDF store")?;
    store
        .load_from_slice(RdfFormat::Turtle, data)
        .context("loading the data graph")?;
    store
        .load_from_slice(RdfFormat::Turtle, ontology)
        .context("loading the ontology")?;
    let shapes_store = Store::new().context("creating the in-memory shapes store")?;
    shapes_store
        .load_from_slice(RdfFormat::Turtle, shapes)
        .context("loading the shapes")?;

    let mut node_shapes = parse_node_shapes(&shapes_store)?;
    let data = Data::new(&store)?;
    for shape in &mut node_shapes {
        shape.prepare_sparql(&data)?;
    }

    let mut report = ShaclReport::default();
    for shape in &node_shapes {
        let focus_nodes = data.focus_nodes(&shape.targets)?;
        report.focus_nodes += focus_nodes.len();
        for focus in &focus_nodes {
            report.results.extend(data.evaluate(shape, focus)?);
        }
    }
    report
        .results
        .sort_by(|a, b| (a.severity, &a.shape, a.line()).cmp(&(b.severity, &b.shape, b.line())));
    Ok(report)
}

// ---------------------------------------------------------------------------
// Shapes

enum Target {
    Class(NamedNode),
    SubjectsOf(NamedNode),
    ObjectsOf(NamedNode),
}

enum ShapePath {
    Predicate(NamedNode),
    Inverse(NamedNode),
}

impl ShapePath {
    fn label(&self) -> String {
        match self {
            ShapePath::Predicate(p) => p.as_str().to_string(),
            ShapePath::Inverse(p) => format!("^{}", p.as_str()),
        }
    }
}

/// The constraint components the d2g shapes use.
enum Constraint {
    MinCount(usize),
    MaxCount(usize),
    Class(NamedNode),
    Datatype(NamedNode),
    MinLength(usize),
    Pattern(Regex),
    NodeKindIri,
    HasValue(Term),
    /// The value nodes are exactly the focus node's values of a predicate.
    Equals(NamedNode),
    Or(Vec<Shape>),
    Sparql(SparqlConstraint),
}

impl Constraint {
    fn component(&self) -> &'static str {
        match self {
            Constraint::MinCount(_) => "MinCountConstraintComponent",
            Constraint::MaxCount(_) => "MaxCountConstraintComponent",
            Constraint::Class(_) => "ClassConstraintComponent",
            Constraint::Datatype(_) => "DatatypeConstraintComponent",
            Constraint::MinLength(_) => "MinLengthConstraintComponent",
            Constraint::Pattern(_) => "PatternConstraintComponent",
            Constraint::NodeKindIri => "NodeKindConstraintComponent",
            Constraint::HasValue(_) => "HasValueConstraintComponent",
            Constraint::Equals(_) => "EqualsConstraintComponent",
            Constraint::Or(_) => "OrConstraintComponent",
            Constraint::Sparql(_) => "SPARQLConstraintComponent",
        }
    }
}

struct SparqlConstraint {
    select: String,
    message: Option<String>,
    prefixes: Vec<(String, String)>,
    /// Focus node -> the `?value` of each solution, filled by
    /// [`Shape::prepare_sparql`]: the query runs once, `$this` unbound, and
    /// each focus node reads its own solutions.
    solutions: HashMap<Term, Vec<Option<Term>>>,
}

struct Shape {
    /// The named node shape this shape is, or belongs to, for reporting.
    owner: String,
    targets: Vec<Target>,
    path: Option<ShapePath>,
    severity: ShaclSeverity,
    message: Option<String>,
    constraints: Vec<Constraint>,
    properties: Vec<Shape>,
}

impl Shape {
    fn prepare_sparql(&mut self, data: &Data<'_>) -> Result<()> {
        for constraint in &mut self.constraints {
            match constraint {
                Constraint::Sparql(sparql) => sparql.solutions = data.run_select(sparql)?,
                Constraint::Or(shapes) => {
                    for shape in shapes {
                        shape.prepare_sparql(data)?;
                    }
                }
                _ => {}
            }
        }
        for property in &mut self.properties {
            property.prepare_sparql(data)?;
        }
        Ok(())
    }
}

fn sh(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{SH}{local}"))
}

/// Every `sh:NodeShape`, in IRI order, so results come in a stable order.
fn parse_node_shapes(shapes: &Store) -> Result<Vec<Shape>> {
    let node_shape = sh("NodeShape");
    let mut names: Vec<NamedOrBlankNode> = Vec::new();
    for quad in shapes.quads_for_pattern(
        None,
        Some(rdf::TYPE),
        Some(node_shape.as_ref().into()),
        None,
    ) {
        names.push(quad?.subject);
    }
    names.sort_by_key(|n| n.to_string());
    names
        .iter()
        .map(|name| parse_shape(shapes, name, &local_name(name)))
        .collect()
}

fn local_name(node: &NamedOrBlankNode) -> String {
    match node {
        NamedOrBlankNode::NamedNode(n) => match n.as_str().rsplit_once(['#', '/']) {
            Some((_, local)) if !local.is_empty() => local.to_string(),
            _ => n.as_str().to_string(),
        },
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

fn parse_shape(shapes: &Store, node: &NamedOrBlankNode, owner: &str) -> Result<Shape> {
    let mut shape = Shape {
        owner: owner.to_string(),
        targets: Vec::new(),
        path: None,
        severity: ShaclSeverity::Violation,
        message: None,
        constraints: Vec::new(),
        properties: Vec::new(),
    };
    for quad in shapes.quads_for_pattern(Some(node.as_ref()), None, None, None) {
        let quad = quad?;
        let predicate = quad.predicate.as_str();
        let Some(local) = predicate.strip_prefix(SH) else {
            continue; // rdf:type, rdfs:comment...: not a SHACL parameter
        };
        let object = &quad.object;
        let at = || format!("{owner}: sh:{local}");
        match local {
            "targetClass" => shape.targets.push(Target::Class(named(object, &at())?)),
            "targetSubjectsOf" => shape
                .targets
                .push(Target::SubjectsOf(named(object, &at())?)),
            "targetObjectsOf" => shape.targets.push(Target::ObjectsOf(named(object, &at())?)),
            "property" => {
                let property = as_subject(object, &at())?;
                let parsed = parse_shape(shapes, &property, owner)?;
                if parsed.path.is_none() {
                    bail!("{owner}: a sh:property without sh:path");
                }
                shape.properties.push(parsed);
            }
            "path" => shape.path = Some(parse_path(shapes, object, &at())?),
            "severity" => {
                shape.severity = match named(object, &at())?.as_str().strip_prefix(SH) {
                    Some("Violation") => ShaclSeverity::Violation,
                    Some("Warning") => ShaclSeverity::Warning,
                    Some("Info") => ShaclSeverity::Info,
                    _ => bail!("{}: unknown severity {object}", at()),
                }
            }
            "message" => shape.message = Some(string(object, &at())?),
            "minCount" => shape
                .constraints
                .push(Constraint::MinCount(count(object, &at())?)),
            "maxCount" => shape
                .constraints
                .push(Constraint::MaxCount(count(object, &at())?)),
            "minLength" => shape
                .constraints
                .push(Constraint::MinLength(count(object, &at())?)),
            "class" => shape
                .constraints
                .push(Constraint::Class(named(object, &at())?)),
            "datatype" => shape
                .constraints
                .push(Constraint::Datatype(named(object, &at())?)),
            "pattern" => {
                let pattern = string(object, &at())?;
                let regex = Regex::new(&pattern)
                    .map_err(|e| anyhow!("{}: invalid pattern {pattern:?}: {e}", at()))?;
                shape.constraints.push(Constraint::Pattern(regex));
            }
            "nodeKind" => {
                if named(object, &at())?.as_str() != format!("{SH}IRI") {
                    bail!("{}: only sh:IRI is supported, not {object}", at());
                }
                shape.constraints.push(Constraint::NodeKindIri);
            }
            "hasValue" => shape.constraints.push(Constraint::HasValue(object.clone())),
            "equals" => shape
                .constraints
                .push(Constraint::Equals(named(object, &at())?)),
            "or" => {
                let members = list(shapes, object, &at())?
                    .iter()
                    .map(|member| parse_shape(shapes, &as_subject(member, &at())?, owner))
                    .collect::<Result<Vec<_>>>()?;
                shape.constraints.push(Constraint::Or(members));
            }
            "sparql" => {
                let constraint = parse_sparql(shapes, &as_subject(object, &at())?, owner)?;
                shape.constraints.push(Constraint::Sparql(constraint));
            }
            _ => bail!(
                "{owner}: sh:{local} is not supported by DITA2Graph's SHACL check \
                 (src/shacl.rs); extend it before using it in the shapes"
            ),
        }
    }
    if shape.path.is_some()
        && shape
            .constraints
            .iter()
            .any(|c| matches!(c, Constraint::Sparql(_)))
    {
        bail!("{owner}: sh:sparql on a property shape is not supported");
    }
    Ok(shape)
}

fn parse_path(shapes: &Store, object: &Term, at: &str) -> Result<ShapePath> {
    match object {
        Term::NamedNode(predicate) => Ok(ShapePath::Predicate(predicate.clone())),
        Term::BlankNode(blank) => {
            let mut inverse = None;
            for quad in shapes.quads_for_pattern(Some(blank.as_ref().into()), None, None, None) {
                let quad = quad?;
                if quad.predicate.as_str() == format!("{SH}inversePath") {
                    inverse = Some(named(&quad.object, at)?);
                } else {
                    bail!("{at}: only a predicate or sh:inversePath of one is supported");
                }
            }
            inverse
                .map(ShapePath::Inverse)
                .ok_or_else(|| anyhow!("{at}: an empty path"))
        }
        _ => bail!("{at}: a path must be an IRI or a blank node"),
    }
}

fn parse_sparql(shapes: &Store, node: &NamedOrBlankNode, owner: &str) -> Result<SparqlConstraint> {
    let mut constraint = SparqlConstraint {
        select: String::new(),
        message: None,
        prefixes: Vec::new(),
        solutions: HashMap::new(),
    };
    for quad in shapes.quads_for_pattern(Some(node.as_ref()), None, None, None) {
        let quad = quad?;
        let at = format!("{owner}: sh:sparql");
        match quad.predicate.as_str().strip_prefix(SH) {
            Some("select") => constraint.select = string(&quad.object, &at)?,
            Some("message") => constraint.message = Some(string(&quad.object, &at)?),
            Some("prefixes") => {
                let declarer = as_subject(&quad.object, &at)?;
                for declare in shapes.quads_for_pattern(
                    Some(declarer.as_ref()),
                    Some(sh("declare").as_ref()),
                    None,
                    None,
                ) {
                    let declaration = as_subject(&declare?.object, &at)?;
                    let get = |local: &str| -> Result<String> {
                        let quad = shapes
                            .quads_for_pattern(
                                Some(declaration.as_ref()),
                                Some(sh(local).as_ref()),
                                None,
                                None,
                            )
                            .next()
                            .ok_or_else(|| anyhow!("{at}: sh:declare without sh:{local}"))??;
                        string(&quad.object, &at)
                    };
                    constraint
                        .prefixes
                        .push((get("prefix")?, get("namespace")?));
                }
            }
            Some(other) => bail!("{at}: sh:{other} is not supported"),
            None => {}
        }
    }
    if constraint.select.is_empty() {
        bail!("{owner}: sh:sparql without sh:select");
    }
    Ok(constraint)
}

fn named(term: &Term, at: &str) -> Result<NamedNode> {
    match term {
        Term::NamedNode(n) => Ok(n.clone()),
        _ => bail!("{at}: expected an IRI, got {term}"),
    }
}

fn as_subject(term: &Term, at: &str) -> Result<NamedOrBlankNode> {
    match term {
        Term::NamedNode(n) => Ok(n.clone().into()),
        Term::BlankNode(b) => Ok(b.clone().into()),
        _ => bail!("{at}: expected an IRI or a blank node, got {term}"),
    }
}

fn string(term: &Term, at: &str) -> Result<String> {
    match term {
        Term::Literal(l) => Ok(l.value().to_string()),
        _ => bail!("{at}: expected a literal, got {term}"),
    }
}

fn count(term: &Term, at: &str) -> Result<usize> {
    string(term, at)?
        .parse()
        .map_err(|e| anyhow!("{at}: expected a non-negative integer: {e}"))
}

/// The members of an RDF list (`rdf:first`/`rdf:rest`).
fn list(shapes: &Store, head: &Term, at: &str) -> Result<Vec<Term>> {
    let mut members = Vec::new();
    let mut cell = head.clone();
    while cell != Term::NamedNode(rdf::NIL.into_owned()) {
        let node = as_subject(&cell, at)?;
        let get = |p: NamedNodeRef<'_>| -> Result<Term> {
            Ok(shapes
                .quads_for_pattern(Some(node.as_ref()), Some(p), None, None)
                .next()
                .ok_or_else(|| anyhow!("{at}: a malformed RDF list"))??
                .object)
        };
        members.push(get(rdf::FIRST)?);
        cell = get(rdf::REST)?;
    }
    Ok(members)
}

// ---------------------------------------------------------------------------
// Data

struct Data<'a> {
    store: &'a Store,
    /// Class -> itself and every class it is a subclass of (rdfs:subClassOf*).
    superclasses: HashMap<NamedNode, HashSet<NamedNode>>,
}

impl<'a> Data<'a> {
    fn new(store: &'a Store) -> Result<Self> {
        let mut direct: HashMap<NamedNode, Vec<NamedNode>> = HashMap::new();
        for quad in store.quads_for_pattern(None, Some(rdfs::SUB_CLASS_OF), None, None) {
            let quad = quad?;
            if let (NamedOrBlankNode::NamedNode(sub), Term::NamedNode(sup)) =
                (quad.subject, quad.object)
            {
                direct.entry(sub).or_default().push(sup);
            }
        }
        let mut superclasses = HashMap::new();
        for class in direct.keys() {
            let mut seen = HashSet::from([class.clone()]);
            let mut stack = vec![class.clone()];
            while let Some(c) = stack.pop() {
                for sup in direct.get(&c).into_iter().flatten() {
                    if seen.insert(sup.clone()) {
                        stack.push(sup.clone());
                    }
                }
            }
            superclasses.insert(class.clone(), seen);
        }
        Ok(Self {
            store,
            superclasses,
        })
    }

    /// A SHACL instance of `class`: an `rdf:type` that is `class` or one of
    /// its subclasses.
    fn is_instance(&self, node: &Term, class: &NamedNode) -> Result<bool> {
        let Some(subject) = subject_ref(node) else {
            return Ok(false);
        };
        for quad in self
            .store
            .quads_for_pattern(Some(subject), Some(rdf::TYPE), None, None)
        {
            if let Term::NamedNode(t) = quad?.object
                && (t == *class || self.superclasses.get(&t).is_some_and(|s| s.contains(class)))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The union of a node shape's targets, each node once, in a stable order.
    fn focus_nodes(&self, targets: &[Target]) -> Result<Vec<Term>> {
        let mut seen = HashSet::new();
        let mut nodes = Vec::new();
        let mut add = |term: Term| {
            if seen.insert(term.clone()) {
                nodes.push(term);
            }
        };
        for target in targets {
            match target {
                Target::Class(class) => {
                    for quad in self
                        .store
                        .quads_for_pattern(None, Some(rdf::TYPE), None, None)
                    {
                        let quad = quad?;
                        if let Term::NamedNode(t) = &quad.object
                            && (t == class
                                || self.superclasses.get(t).is_some_and(|s| s.contains(class)))
                        {
                            add(quad.subject.into());
                        }
                    }
                }
                Target::SubjectsOf(p) => {
                    for quad in self
                        .store
                        .quads_for_pattern(None, Some(p.as_ref()), None, None)
                    {
                        add(quad?.subject.into());
                    }
                }
                Target::ObjectsOf(p) => {
                    for quad in self
                        .store
                        .quads_for_pattern(None, Some(p.as_ref()), None, None)
                    {
                        add(quad?.object);
                    }
                }
            }
        }
        nodes.sort_by_key(|t| t.to_string());
        Ok(nodes)
    }

    fn values(&self, path: &ShapePath, focus: &Term) -> Result<Vec<Term>> {
        let mut values = Vec::new();
        match path {
            ShapePath::Predicate(p) => {
                if let Some(subject) = subject_ref(focus) {
                    for quad in
                        self.store
                            .quads_for_pattern(Some(subject), Some(p.as_ref()), None, None)
                    {
                        values.push(quad?.object);
                    }
                }
            }
            ShapePath::Inverse(p) => {
                for quad in
                    self.store
                        .quads_for_pattern(None, Some(p.as_ref()), Some(focus.as_ref()), None)
                {
                    values.push(quad?.subject.into());
                }
            }
        }
        Ok(values)
    }

    /// Runs a SPARQL constraint once, `$this` unbound, and groups the
    /// solutions by `$this`: the same results as one run per focus node with
    /// `$this` pre-bound, for queries that join on `$this` as these do.
    fn run_select(&self, sparql: &SparqlConstraint) -> Result<HashMap<Term, Vec<Option<Term>>>> {
        let mut evaluator = SparqlEvaluator::new();
        for (prefix, namespace) in &sparql.prefixes {
            evaluator = evaluator
                .with_prefix(prefix, namespace)
                .map_err(|e| anyhow!("SPARQL prefix {prefix}: {e}"))?;
        }
        let query = evaluator
            .parse_query(&sparql.select)
            .map_err(|e| anyhow!("SPARQL constraint {:?}: {e}", sparql.select.trim()))?;
        let mut solutions: HashMap<Term, Vec<Option<Term>>> = HashMap::new();
        match query.on_store(self.store).execute()? {
            QueryResults::Solutions(rows) => {
                for row in rows {
                    let row = row?;
                    if let Some(this) = row.get("this") {
                        solutions
                            .entry(this.clone())
                            .or_default()
                            .push(row.get("value").cloned());
                    }
                }
            }
            _ => bail!("a SPARQL constraint must be a SELECT query"),
        }
        Ok(solutions)
    }

    /// Validates `focus` against `shape`: its own constraints on its value
    /// nodes (the focus node itself, or the values of its path), then each
    /// property shape on each value node.
    fn evaluate(&self, shape: &Shape, focus: &Term) -> Result<Vec<ShaclResult>> {
        let values = match &shape.path {
            Some(path) => self.values(path, focus)?,
            None => vec![focus.clone()],
        };
        let mut results = Vec::new();
        for constraint in &shape.constraints {
            for value in self.violations(constraint, focus, &values)? {
                results.push(ShaclResult {
                    severity: shape.severity,
                    component: constraint.component(),
                    shape: shape.owner.clone(),
                    focus: focus.clone(),
                    path: shape.path.as_ref().map(ShapePath::label),
                    message: message(shape, constraint, value.as_ref()),
                    value,
                });
            }
        }
        for property in &shape.properties {
            for value in &values {
                results.extend(self.evaluate(property, value)?);
            }
        }
        Ok(results)
    }

    /// Whether `node` conforms to `shape` (a member of `sh:or`).
    fn conforms(&self, shape: &Shape, node: &Term) -> Result<bool> {
        Ok(self.evaluate(shape, node)?.is_empty())
    }

    /// One entry per result the constraint produces: the offending value
    /// node, or none for a constraint on the value nodes as a whole.
    fn violations(
        &self,
        constraint: &Constraint,
        focus: &Term,
        values: &[Term],
    ) -> Result<Vec<Option<Term>>> {
        let each = |test: &dyn Fn(&Term) -> Result<bool>| -> Result<Vec<Option<Term>>> {
            let mut bad = Vec::new();
            for value in values {
                if !test(value)? {
                    bad.push(Some(value.clone()));
                }
            }
            Ok(bad)
        };
        Ok(match constraint {
            Constraint::MinCount(n) => once(values.len() < *n),
            Constraint::MaxCount(n) => once(values.len() > *n),
            Constraint::HasValue(expected) => once(!values.contains(expected)),
            // Each value on one side only is a result, as SHACL defines it.
            Constraint::Equals(predicate) => {
                let other = self.values(&ShapePath::Predicate(predicate.clone()), focus)?;
                let mut bad: Vec<Option<Term>> = values
                    .iter()
                    .filter(|v| !other.contains(v))
                    .chain(other.iter().filter(|v| !values.contains(v)))
                    .cloned()
                    .map(Some)
                    .collect();
                bad.dedup();
                bad
            }
            Constraint::Class(class) => each(&|v| self.is_instance(v, class))?,
            Constraint::Datatype(datatype) => {
                each(&|v| Ok(matches!(v, Term::Literal(l) if l.datatype() == datatype.as_ref())))?
            }
            Constraint::MinLength(n) => {
                each(&|v| Ok(lexical(v).is_some_and(|s| s.chars().count() >= *n)))?
            }
            Constraint::Pattern(regex) => {
                each(&|v| Ok(lexical(v).is_some_and(|s| regex.is_match(s))))?
            }
            Constraint::NodeKindIri => each(&|v| Ok(matches!(v, Term::NamedNode(_))))?,
            Constraint::Or(shapes) => each(&|v| {
                for shape in shapes {
                    if self.conforms(shape, v)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            })?,
            // pySHACL reports a solution without ?value with the focus node
            // as its value; doing the same keeps the two comparable.
            Constraint::Sparql(sparql) => sparql
                .solutions
                .get(focus)
                .into_iter()
                .flatten()
                .map(|value| Some(value.clone().unwrap_or_else(|| focus.clone())))
                .collect(),
        })
    }
}

fn once(failed: bool) -> Vec<Option<Term>> {
    if failed { vec![None] } else { Vec::new() }
}

fn subject_ref(term: &Term) -> Option<NamedOrBlankNodeRef<'_>> {
    match term {
        Term::NamedNode(n) => Some(n.as_ref().into()),
        Term::BlankNode(b) => Some(b.as_ref().into()),
        _ => None,
    }
}

/// An IRI's or a literal's string, as `sh:minLength` and `sh:pattern` read
/// it; none for a blank node, which both reject.
fn lexical(term: &Term) -> Option<&str> {
    match term.as_ref() {
        TermRef::NamedNode(n) => Some(n.as_str()),
        TermRef::Literal(l) => Some(l.value()),
        _ => None,
    }
}

/// A term as the result lines show it: an IRI bare, a literal quoted.
fn plain(term: &Term) -> String {
    match term {
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::Literal(l) => format!("\"{}\"", l.value()),
        other => other.to_string(),
    }
}

/// `sh:message` (the SPARQL constraint's own, then the shape's), or a
/// description of the component.
fn message(shape: &Shape, constraint: &Constraint, value: Option<&Term>) -> String {
    let own = match constraint {
        Constraint::Sparql(sparql) => sparql.message.clone(),
        _ => None,
    };
    if let Some(message) = own.or_else(|| shape.message.clone()) {
        return message;
    }
    let path = shape
        .path
        .as_ref()
        .map(ShapePath::label)
        .unwrap_or_else(|| "the node".into());
    let value = value.map(plain).unwrap_or_default();
    match constraint {
        Constraint::MinCount(n) => format!("fewer than {n} value(s) for {path}"),
        Constraint::MaxCount(n) => format!("more than {n} value(s) for {path}"),
        Constraint::Class(class) => format!("{value} ({path}) is not a {}", class.as_str()),
        Constraint::Datatype(d) => format!("{value} ({path}) is not a {}", d.as_str()),
        Constraint::MinLength(n) => format!("{value} ({path}) is shorter than {n}"),
        Constraint::Pattern(regex) => format!("{value} ({path}) does not match {}", regex.as_str()),
        Constraint::NodeKindIri => format!("{value} ({path}) is not an IRI"),
        Constraint::HasValue(v) => format!("{path} lacks the value {}", plain(v)),
        Constraint::Equals(p) => format!("{value} is a value of {path} or {} only", p.as_str()),
        Constraint::Or(_) => format!("{value} ({path}) matches none of the sh:or shapes"),
        Constraint::Sparql(_) => "a SPARQL constraint failed".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::{ONTOLOGY_TTL, SHAPES_TTL};

    const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data");

    fn check(file: &str) -> ShaclReport {
        let data = fs::read(format!("{DATA}/{file}")).unwrap();
        validate(&data, ONTOLOGY_TTL.as_bytes(), SHAPES_TTL.as_bytes()).unwrap()
    }

    fn lines(report: &ShaclReport) -> Vec<String> {
        let mut lines: Vec<String> = report.results.iter().map(ShaclResult::line).collect();
        lines.sort();
        lines
    }

    #[test]
    fn the_golden_export_conforms_without_a_single_result() {
        let report = check("rdf-model.graph.ttl");
        assert!(report.results.is_empty(), "{:#?}", report.results);
        assert!(report.focus_nodes > 0);
    }

    #[test]
    fn the_invalid_graph_gets_exactly_pyshacls_results() {
        // tests/data/rdf-invalid.shacl-results.txt is pySHACL 0.30.1's own
        // report on the same graph (-i none, ontology as ont_graph), one
        // `severity|component|focus|path|value` line per result.
        let expected: Vec<String> =
            fs::read_to_string(format!("{DATA}/rdf-invalid.shacl-results.txt"))
                .unwrap()
                .lines()
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect();
        let report = check("rdf-invalid.ttl");
        assert_eq!(lines(&report), expected);
        assert!(!report.passes());
    }

    #[test]
    fn a_property_shape_keeps_its_own_severity_and_only_violations_fail() {
        const PREFIXES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
                                @prefix d2g: <https://w3id.org/dita2graph/ns#> .\n";
        let data = format!("{PREFIXES}<https://example.org/m> a d2g:Map .\n");
        let check = |shape: &str| {
            let shapes = format!("{PREFIXES}<https://example.org/s> a sh:NodeShape ; {shape} .\n");
            validate(data.as_bytes(), ONTOLOGY_TTL.as_bytes(), shapes.as_bytes()).unwrap()
        };

        // A warning on the node shape does not reach its property shapes:
        // SHACL gives each shape its own severity, Violation by default.
        let report = check(
            "sh:targetClass d2g:Node ; sh:severity sh:Warning ; \
             sh:property [ sh:path d2g:product ; sh:minCount 1 ]",
        );
        assert_eq!(
            report.count(ShaclSeverity::Violation),
            1,
            "{:#?}",
            report.results
        );
        assert!(!report.passes());

        let report = check(
            "sh:targetClass d2g:Node ; \
             sh:property [ sh:path d2g:product ; sh:minCount 1 ; sh:severity sh:Info ]",
        );
        assert_eq!(
            report.count(ShaclSeverity::Info),
            1,
            "{:#?}",
            report.results
        );
        assert!(report.passes());
    }

    /// Parity with pySHACL on a real export: `DITA2GRAPH_SHACL_RDF_DIR`
    /// (a bundle's `rdf/`) and `DITA2GRAPH_SHACL_EXPECTED` (pySHACL's
    /// results for it, in the `line()` form). Run with `--ignored`.
    #[test]
    #[ignore = "needs a real export and pySHACL's results for it"]
    fn matches_pyshacl_on_a_real_export() {
        let rdf_dir = std::env::var("DITA2GRAPH_SHACL_RDF_DIR").unwrap();
        let expected: Vec<String> =
            fs::read_to_string(std::env::var("DITA2GRAPH_SHACL_EXPECTED").unwrap())
                .unwrap()
                .lines()
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
        let started = std::time::Instant::now();
        let report = validate_rdf_dir(Path::new(&rdf_dir)).unwrap();
        eprintln!(
            "{} results on {} focus nodes in {:.2} s",
            report.results.len(),
            report.focus_nodes,
            started.elapsed().as_secs_f64()
        );
        assert_eq!(lines(&report), expected);
    }

    #[test]
    fn an_unsupported_shacl_feature_is_an_error_not_a_pass() {
        let shapes = format!(
            "{SHAPES_TTL}\n<https://example.org/s> a sh:NodeShape ; sh:targetClass d2g:Map ; \
             sh:closed true .\n"
        );
        let err = validate(b"", ONTOLOGY_TTL.as_bytes(), shapes.as_bytes()).unwrap_err();
        assert!(
            err.to_string().contains("sh:closed is not supported"),
            "{err}"
        );
    }
}
