use crate::{
    SearchCache, SearchOptions, SegmentKind, SegmentMatcher, SlabIndex, build_segment_matchers,
    cache::NAME_POOL,
};
use anyhow::{Result, anyhow, bail};
use cardinal_syntax::{ArgumentKind, Expr, Filter, FilterArgument, FilterKind, Term};
use fswalk::NodeFileType;
use hashbrown::HashSet;
use query_segmentation::query_segmentation;
use regex::RegexBuilder;
use search_cancel::CancellationToken;
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

const CANCEL_CHECK_INTERVAL: usize = 0x10000;

impl SearchCache {
    pub(crate) fn evaluate_expr(
        &self,
        expr: &Expr,
        options: SearchOptions,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        match expr {
            Expr::Empty => Ok(self.search_empty(token)),
            Expr::Term(term) => self.evaluate_term(term, options, token),
            Expr::Not(inner) => self.evaluate_not(inner, None, options, token),
            Expr::And(parts) => self.evaluate_and(parts, options, token),
            Expr::Or(parts) => self.evaluate_or(parts, options, token),
        }
    }

    fn evaluate_and(
        &self,
        parts: &[Expr],
        options: SearchOptions,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        if parts.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let mut current: Option<Vec<SlabIndex>> = None;
        for part in parts {
            match part {
                Expr::Empty => {}
                Expr::Not(inner) => {
                    current = self.evaluate_not(inner, current, options, token)?;
                }
                _ => {
                    let Some(nodes) = self.evaluate_expr(part, options, token)? else {
                        return Ok(None);
                    };
                    current = Some(match current {
                        Some(mut existing) => {
                            if intersect_in_place(&mut existing, &nodes, token).is_none() {
                                return Ok(None);
                            }
                            existing
                        }
                        None => nodes,
                    });
                }
            }
        }
        Ok(Some(current.unwrap_or_default()))
    }

    fn evaluate_or(
        &self,
        parts: &[Expr],
        options: SearchOptions,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        if parts.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let mut result: Vec<SlabIndex> = Vec::new();
        for part in parts {
            let candidate = self.evaluate_expr(part, options, token)?;
            let Some(nodes) = candidate else {
                return Ok(None);
            };
            if union_in_place(&mut result, &nodes, token).is_none() {
                return Ok(None);
            }
        }
        Ok(Some(result))
    }

    fn evaluate_not(
        &self,
        inner: &Expr,
        base: Option<Vec<SlabIndex>>,
        options: SearchOptions,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        let mut universe = if let Some(current) = base {
            current
        } else {
            match self.search_empty(token) {
                Some(nodes) => nodes,
                None => return Ok(None),
            }
        };
        if let Some(negated) = self.evaluate_expr(inner, options, token)? {
            if difference_in_place(&mut universe, &negated, token).is_none() {
                return Ok(None);
            }
        } else {
            return Ok(None);
        }
        Ok(Some(universe))
    }

    fn evaluate_term(
        &self,
        term: &Term,
        options: SearchOptions,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        match term {
            Term::Word(text) | Term::Phrase(text) => self.evaluate_word(text, options, token),
            Term::Regex(pattern) => self.evaluate_regex(pattern, options, token),
            Term::Filter(filter) => self.evaluate_filter(filter, options, token),
        }
    }

    fn evaluate_word(
        &self,
        text: &str,
        options: SearchOptions,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        let segments = query_segmentation(text);
        if segments.is_empty() {
            bail!("Unprocessable term: {text:?}");
        }
        let matchers = build_segment_matchers(&segments, options)
            .map_err(|err| anyhow!("Invalid regex pattern: {err}"))?;
        self.execute_matchers(&matchers, token)
    }

    fn execute_matchers(
        &self,
        matchers: &[SegmentMatcher],
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        if matchers.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let mut node_set: Option<Vec<SlabIndex>> = None;
        for matcher in matchers {
            if let Some(nodes) = &node_set {
                let mut new_node_set = Vec::with_capacity(nodes.len());
                for (i, &node) in nodes.iter().enumerate() {
                    if i % CANCEL_CHECK_INTERVAL == 0 && token.is_cancelled() {
                        return Ok(None);
                    }
                    let mut child_matches = self.file_nodes[node]
                        .children
                        .iter()
                        .filter_map(|&child| {
                            let name = self.file_nodes[child].name_and_parent.as_str();
                            if matcher.matches(name) {
                                Some((name, child))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    child_matches.sort_unstable_by_key(|(name, _)| *name);
                    new_node_set.extend(child_matches.into_iter().map(|(_, index)| index));
                }
                node_set = Some(new_node_set);
            } else {
                let names: Option<BTreeSet<_>> = match matcher {
                    SegmentMatcher::Plain { kind, needle } => match kind {
                        SegmentKind::Substr => NAME_POOL.search_substr(needle, token),
                        SegmentKind::Prefix => NAME_POOL.search_prefix(needle, token),
                        SegmentKind::Suffix => NAME_POOL.search_suffix(needle, token),
                        SegmentKind::Exact => NAME_POOL.search_exact(needle, token),
                    },
                    SegmentMatcher::Regex { regex } => NAME_POOL.search_regex(regex, token),
                };
                let Some(names) = names else {
                    return Ok(None);
                };
                let mut nodes = Vec::with_capacity(names.len());
                for (i, name) in names.iter().enumerate() {
                    if i % CANCEL_CHECK_INTERVAL == 0 && token.is_cancelled() {
                        return Ok(None);
                    }
                    if let Some(indices) = self.name_index.get(name) {
                        nodes.extend(indices.iter().copied());
                    }
                }
                node_set = Some(nodes);
            }
        }
        Ok(node_set)
    }

    fn evaluate_regex(
        &self,
        pattern: &str,
        options: SearchOptions,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        let mut builder = RegexBuilder::new(pattern);
        builder.case_insensitive(options.case_insensitive);
        let regex = builder
            .build()
            .map_err(|err| anyhow!("Invalid regex pattern: {err}"))?;
        let matcher = SegmentMatcher::Regex { regex };
        self.execute_matchers(std::slice::from_ref(&matcher), token)
    }

    fn evaluate_filter(
        &self,
        filter: &Filter,
        options: SearchOptions,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        match filter.kind {
            FilterKind::File => self.evaluate_type_filter(
                NodeFileType::File,
                filter.argument.as_ref(),
                options,
                token,
            ),
            FilterKind::Folder => self.evaluate_type_filter(
                NodeFileType::Dir,
                filter.argument.as_ref(),
                options,
                token,
            ),
            FilterKind::Ext => {
                let argument = filter
                    .argument
                    .as_ref()
                    .ok_or_else(|| anyhow!("ext: requires at least one extension"))?;
                self.evaluate_extension_filter(argument, token)
            }
            FilterKind::Parent => {
                let argument = filter
                    .argument
                    .as_ref()
                    .ok_or_else(|| anyhow!("parent: requires a folder path"))?;
                self.evaluate_parent_filter(argument, token)
            }
            FilterKind::InFolder => {
                let argument = filter
                    .argument
                    .as_ref()
                    .ok_or_else(|| anyhow!("infolder: requires a folder path"))?;
                self.evaluate_infolder_filter(argument, token)
            }
            _ => bail!("Filter {:?} is not supported yet", filter.kind),
        }
    }

    fn evaluate_type_filter(
        &self,
        file_type: NodeFileType,
        argument: Option<&FilterArgument>,
        options: SearchOptions,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        let base = if let Some(arg) = argument {
            self.evaluate_word(&arg.raw, options, token)?
        } else {
            self.search_empty(token)
        };
        let Some(nodes) = base else {
            return Ok(None);
        };
        Ok(filter_nodes(nodes, token, |index| {
            self.file_nodes[index].metadata.file_type_hint() == file_type
        }))
    }

    fn evaluate_extension_filter(
        &self,
        argument: &FilterArgument,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        let extensions = normalize_extensions(argument);
        if extensions.is_empty() {
            bail!("ext: requires non-empty extensions");
        }
        let Some(nodes) = self.search_empty(token) else {
            return Ok(None);
        };
        Ok(filter_nodes(nodes, token, |index| {
            let node = &self.file_nodes[index];
            if node.metadata.file_type_hint() != NodeFileType::File {
                return false;
            }
            extension_of(node.name_and_parent.as_str())
                .map(|ext| extensions.contains(ext.as_str()))
                .unwrap_or(false)
        }))
    }

    fn evaluate_parent_filter(
        &self,
        argument: &FilterArgument,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        let target = self.resolve_query_path(&argument.raw);
        let Some(nodes) = self.search_empty(token) else {
            return Ok(None);
        };
        Ok(filter_nodes(nodes, token, |index| {
            self.node_parent_path(index)
                .map(|parent| parent == target)
                .unwrap_or(false)
        }))
    }

    fn evaluate_infolder_filter(
        &self,
        argument: &FilterArgument,
        token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        let target = self.resolve_query_path(&argument.raw);
        let Some(nodes) = self.search_empty(token) else {
            return Ok(None);
        };
        Ok(filter_nodes(nodes, token, |index| {
            self.file_nodes
                .node_path(index)
                .is_some_and(|path| path.starts_with(&target) && path != target)
        }))
    }

    fn node_parent_path(&self, index: SlabIndex) -> Option<PathBuf> {
        let parent = self.file_nodes[index].name_and_parent.parent()?;
        self.file_nodes.node_path(parent)
    }

    fn resolve_query_path(&self, raw: &str) -> PathBuf {
        let trimmed = raw.trim();
        let candidate = PathBuf::from(trimmed);
        let joined = if candidate.is_absolute() {
            candidate
        } else {
            self.file_nodes.path().join(candidate)
        };
        normalize_path(&joined)
    }
}

fn normalize_extensions(argument: &FilterArgument) -> HashSet<String> {
    let mut values = HashSet::new();
    match &argument.kind {
        ArgumentKind::List(list) => {
            for item in list {
                if let Some(ext) = normalize_extension(item) {
                    values.insert(ext);
                }
            }
        }
        _ => {
            if let Some(ext) = normalize_extension(&argument.raw) {
                values.insert(ext);
            }
        }
    }
    values
}

fn normalize_extension(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
    }
}

fn extension_of(name: &str) -> Option<String> {
    let pos = name.rfind('.')?;
    if pos + 1 >= name.len() {
        return None;
    }
    Some(name[pos + 1..].to_ascii_lowercase())
}

fn filter_nodes(
    nodes: Vec<SlabIndex>,
    token: CancellationToken,
    mut predicate: impl FnMut(SlabIndex) -> bool,
) -> Option<Vec<SlabIndex>> {
    let mut filtered = Vec::with_capacity(nodes.len());
    for (i, index) in nodes.into_iter().enumerate() {
        if i % CANCEL_CHECK_INTERVAL == 0 && token.is_cancelled() {
            return None;
        }
        if predicate(index) {
            filtered.push(index);
        }
    }
    Some(filtered)
}

fn intersect_in_place(
    values: &mut Vec<SlabIndex>,
    rhs: &[SlabIndex],
    token: CancellationToken,
) -> Option<()> {
    if values.is_empty() {
        return Some(());
    }
    let rhs_set: HashSet<SlabIndex> = rhs.iter().copied().collect();
    let mut filtered = Vec::with_capacity(values.len().min(rhs.len()));
    for (i, index) in values.iter().copied().enumerate() {
        if i % CANCEL_CHECK_INTERVAL == 0 && token.is_cancelled() {
            return None;
        }
        if rhs_set.contains(&index) {
            filtered.push(index);
        }
    }
    *values = filtered;
    Some(())
}

fn difference_in_place(
    values: &mut Vec<SlabIndex>,
    rhs: &[SlabIndex],
    token: CancellationToken,
) -> Option<()> {
    if values.is_empty() || rhs.is_empty() {
        return Some(());
    }
    let rhs_set: HashSet<SlabIndex> = rhs.iter().copied().collect();
    let mut filtered = Vec::with_capacity(values.len());
    for (i, index) in values.iter().copied().enumerate() {
        if i % CANCEL_CHECK_INTERVAL == 0 && token.is_cancelled() {
            return None;
        }
        if !rhs_set.contains(&index) {
            filtered.push(index);
        }
    }
    *values = filtered;
    Some(())
}

fn union_in_place(
    values: &mut Vec<SlabIndex>,
    rhs: &[SlabIndex],
    token: CancellationToken,
) -> Option<()> {
    if rhs.is_empty() {
        return Some(());
    }
    let mut seen: HashSet<SlabIndex> = values.iter().copied().collect();
    for (i, index) in rhs.iter().copied().enumerate() {
        if i % CANCEL_CHECK_INTERVAL == 0 && token.is_cancelled() {
            return None;
        }
        if seen.insert(index) {
            values.push(index);
        }
    }
    Some(())
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::Prefix(prefix) => {
                normalized.push(prefix.as_os_str());
            }
        }
    }
    normalized
}
