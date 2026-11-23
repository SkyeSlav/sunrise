//! The ordered maps in advancement save JSON. DFU maps names in encounter
//! order, then `JsonOps.createMap` keeps the last value written at a duplicate
//! key. Keep the pairs at the JSON boundary so NBT's HashMap order cannot choose
//! which progress survives an advancement ID or criterion rename.
use super::files::{AdvancementJson, json_to_tag, tag_to_json};
use super::json::Json;
use sunrise_core::nbt::{JavaString, Tag};

pub(super) fn read(value: &Json, value_context: AdvancementJson) -> Tag {
    match value {
        Json::Obj(entries) => Tag::List(vec![
            Tag::LongArray(Vec::new()),
            Tag::List(
                entries
                    .iter()
                    .map(|(k, v)| {
                        Tag::List(vec![
                            Tag::String(JavaString(k.clone())),
                            json_to_tag(v, value_context),
                        ])
                    })
                    .collect(),
            ),
        ]),
        other => json_to_tag(other, AdvancementJson::Other),
    }
}

// JSON arrays become lists, not LongArrays, so the marker cannot appear in
// decoded JSON. Non-object criteria stay opaque to the key transform.
fn pairs(criteria: &mut Tag) -> Option<&mut Vec<Tag>> {
    if let Tag::List(marker) = criteria
        && let [Tag::LongArray(_), Tag::List(pairs)] = marker.as_mut_slice()
    {
        Some(pairs)
    } else {
        None
    }
}

pub(super) fn map_keys(criteria: &mut Tag, mut rename: impl FnMut(JavaString) -> JavaString) {
    if let Some(pairs) = pairs(criteria) {
        for entry in pairs {
            if let Tag::List(parts) = entry
                && let [Tag::String(key), _] = parts.as_mut_slice()
            {
                *key = rename(std::mem::take(key));
            }
        }
    } else if let Tag::Compound(c) = criteria {
        for (key, value) in std::mem::take(c) {
            c.insert(rename(key), value);
        }
    }
}

/// Look up one advancement with registered typed criteria, from either input form.
pub(super) fn criteria<'a>(root: &'a mut Tag, advancement: &str) -> Option<&'a mut Tag> {
    if matches!(root, Tag::Compound(_)) {
        return root.get_mut(advancement)?.get_mut("criteria");
    }
    for pair in pairs(root)? {
        if let Tag::List(parts) = pair
            && let [Tag::String(key), entry] = parts.as_mut_slice()
            && key.0.iter().copied().eq(advancement.encode_utf16())
        {
            return entry.get_mut("criteria");
        }
    }
    None
}

/// `AdvancementsRenameFix` rewrites a JSON map rather than a typed list of
/// pairs. Collapse duplicates at each rewrite: the last value wins at the
/// first key's position, which can matter at a subsequent rename.
pub(super) fn rename_advancements(root: &mut Tag, rename: impl FnMut(JavaString) -> JavaString) {
    map_keys(root, rename);
    if let Some(pairs) = pairs(root) {
        let mut slots = std::collections::HashMap::with_capacity(pairs.len());
        let mut ordered = Vec::with_capacity(pairs.len());
        for pair in std::mem::take(pairs) {
            let Tag::List(parts) = &pair else { continue };
            let [Tag::String(key), _] = parts.as_slice() else {
                continue;
            };
            let slot = *slots.entry(key.clone()).or_insert(ordered.len());
            if slot == ordered.len() {
                ordered.push(pair);
            } else {
                ordered[slot] = pair;
            }
        }
        *pairs = ordered;
    }
}

pub(super) fn write(value: &Tag, value_context: AdvancementJson) -> Json {
    if let Tag::List(marker) = value
        && let [Tag::LongArray(_), Tag::List(pairs)] = marker.as_slice()
    {
        let mut entries = Vec::with_capacity(pairs.len());
        for entry in pairs {
            if let Tag::List(parts) = entry
                && let [Tag::String(key), value] = parts.as_slice()
            {
                Json::insert(
                    &mut entries,
                    key.0.clone(),
                    tag_to_json(value, value_context),
                );
            }
        }
        Json::Obj(entries)
    } else {
        tag_to_json(value, AdvancementJson::Other)
    }
}
