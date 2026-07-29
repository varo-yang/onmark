//! Source-tree collection of presentation bindings and their semantic owners.
//!
//! This pass observes presentation markup before structural binding consumes the
//! syntax tree. It records source facts only; field lookup and fallback
//! validation remain in resolution.

use crate::diagnostics::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::model::ElementKind;
use crate::syntax::{Attribute, Element, Node};

use super::diagnostic::author_diagnostic;
use super::variant::{
    LinkedVariantBinding, LinkedVariantFallback, LinkedVariantScope, VariantBindingSink,
};

pub(super) fn collect_variant_bindings(
    film: &Element,
    diagnostics: &mut Diagnostics,
) -> Vec<LinkedVariantBinding> {
    let mut collector = BindingCollector {
        bindings: Vec::new(),
        diagnostics,
    };
    collector.collect_element(film, LinkedVariantScope::Film, false);
    collector.bindings
}

struct BindingCollector<'a> {
    bindings: Vec<LinkedVariantBinding>,
    diagnostics: &'a mut Diagnostics,
}

impl BindingCollector<'_> {
    fn collect_element(
        &mut self,
        element: &Element,
        inherited_scope: LinkedVariantScope,
        inherited_forbidden: bool,
    ) {
        let kind = ElementKind::from_local_name(element.name().local());
        let scope = semantic_scope(kind, element, inherited_scope);
        let forbidden = inherited_forbidden || forbids_bindings(kind, element);

        self.collect_attributes(element, scope, forbidden);

        for child in element.children() {
            if let Node::Element(child) = child {
                self.collect_element(child, scope, forbidden);
            }
        }
    }

    fn collect_attributes(
        &mut self,
        element: &Element,
        scope: LinkedVariantScope,
        forbidden: bool,
    ) {
        for attribute in element.attributes() {
            let Some(sink) = binding_sink(attribute) else {
                continue;
            };
            if forbidden {
                self.diagnostics.push(forbidden_binding(attribute, element));
                continue;
            }

            self.bindings.push(LinkedVariantBinding::new(
                sink,
                attribute.clone(),
                fallback(element, sink),
                scope,
                element.span(),
            ));
        }
    }
}

fn semantic_scope(
    kind: Option<ElementKind>,
    element: &Element,
    inherited: LinkedVariantScope,
) -> LinkedVariantScope {
    match kind {
        Some(ElementKind::Film) => LinkedVariantScope::Film,
        Some(ElementKind::Scene) => LinkedVariantScope::Scene(element.span()),
        Some(ElementKind::Shot) => LinkedVariantScope::Shot(element.span()),
        Some(ElementKind::Transition) => LinkedVariantScope::Transition(element.span()),
        _ => inherited,
    }
}

fn forbids_bindings(kind: Option<ElementKind>, element: &Element) -> bool {
    matches!(
        kind,
        Some(
            ElementKind::Fields
                | ElementKind::Field
                | ElementKind::Cues
                | ElementKind::Cue
                | ElementKind::VoiceOver
                | ElementKind::Music
                | ElementKind::SoundEffect
        )
    ) || matches!(element.name().local(), "script" | "style")
}

fn binding_sink(attribute: &Attribute) -> Option<VariantBindingSink> {
    match attribute.name().local() {
        "data-om-text" => Some(VariantBindingSink::Text),
        "data-om-css" => Some(VariantBindingSink::Css),
        "data-om-show" => Some(VariantBindingSink::Show),
        _ => None,
    }
}

fn fallback(element: &Element, sink: VariantBindingSink) -> LinkedVariantFallback {
    match sink {
        VariantBindingSink::Text => text_fallback(element),
        VariantBindingSink::Css => LinkedVariantFallback::Css {
            style: find_attribute(element, "style").cloned(),
        },
        VariantBindingSink::Show => LinkedVariantFallback::Show {
            hidden: find_attribute(element, "hidden").is_some(),
        },
    }
}

fn text_fallback(element: &Element) -> LinkedVariantFallback {
    let mut value = String::new();
    let mut direct_text_only = true;

    for child in element.children() {
        match child {
            Node::Text(text) => value.push_str(text.text()),
            Node::Element(_) => direct_text_only = false,
        }
    }

    LinkedVariantFallback::Text {
        value: value.into(),
        direct_text_only,
    }
}

fn find_attribute<'a>(element: &'a Element, name: &str) -> Option<&'a Attribute> {
    element
        .attributes()
        .iter()
        .find(|attribute| attribute.name().local() == name)
}

fn forbidden_binding(attribute: &Attribute, element: &Element) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::IncompatibleVariantBinding,
        attribute.name().span(),
        format!(
            "attribute \"{}\" is not allowed on or inside <{}>",
            attribute.name(),
            element.name(),
        ),
        "move the binding to presentation markup owned by a film, scene, shot, or transition",
    )
}
