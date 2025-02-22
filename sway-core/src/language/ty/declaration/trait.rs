use std::hash::{Hash, Hasher};

use sway_error::handler::{ErrorEmitted, Handler};
use sway_types::{Ident, Named, Span, Spanned};

use crate::{
    decl_engine::{
        mapping::DeclMapping, DeclEngineReplace, DeclRefConstant, DeclRefFunction, DeclRefTraitFn,
        DeclRefTraitType, ReplaceFunctionImplementingType,
    },
    engine_threading::*,
    language::{parsed, Visibility},
    semantic_analysis::type_check_context::MonomorphizeHelper,
    semantic_analysis::{TypeCheckFinalization, TypeCheckFinalizationContext},
    transform,
    type_system::*,
};

use super::TyDecl;

#[derive(Clone, Debug)]
pub struct TyTraitDecl {
    pub name: Ident,
    pub type_parameters: Vec<TypeParameter>,
    pub self_type: TypeParameter,
    pub interface_surface: Vec<TyTraitInterfaceItem>,
    pub items: Vec<TyTraitItem>,
    pub supertraits: Vec<parsed::Supertrait>,
    pub visibility: Visibility,
    pub attributes: transform::AttributesMap,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TyTraitInterfaceItem {
    TraitFn(DeclRefTraitFn),
    Constant(DeclRefConstant),
    Type(DeclRefTraitType),
}

#[derive(Clone, Debug)]
pub enum TyTraitItem {
    Fn(DeclRefFunction),
    Constant(DeclRefConstant),
    Type(DeclRefTraitType),
}

impl Named for TyTraitDecl {
    fn name(&self) -> &Ident {
        &self.name
    }
}

impl Spanned for TyTraitDecl {
    fn span(&self) -> Span {
        self.span.clone()
    }
}

impl EqWithEngines for TyTraitDecl {}
impl PartialEqWithEngines for TyTraitDecl {
    fn eq(&self, other: &Self, engines: &Engines) -> bool {
        self.name == other.name
            && self.type_parameters.eq(&other.type_parameters, engines)
            && self.interface_surface.eq(&other.interface_surface, engines)
            && self.items.eq(&other.items, engines)
            && self.supertraits.eq(&other.supertraits, engines)
            && self.visibility == other.visibility
    }
}

impl HashWithEngines for TyTraitDecl {
    fn hash<H: Hasher>(&self, state: &mut H, engines: &Engines) {
        let TyTraitDecl {
            name,
            type_parameters,
            self_type,
            interface_surface,
            items,
            supertraits,
            visibility,
            // these fields are not hashed because they aren't relevant/a
            // reliable source of obj v. obj distinction
            attributes: _,
            span: _,
        } = self;
        name.hash(state);
        type_parameters.hash(state, engines);
        self_type.hash(state, engines);
        interface_surface.hash(state, engines);
        items.hash(state, engines);
        supertraits.hash(state, engines);
        visibility.hash(state);
    }
}

impl EqWithEngines for TyTraitInterfaceItem {}
impl PartialEqWithEngines for TyTraitInterfaceItem {
    fn eq(&self, other: &Self, engines: &Engines) -> bool {
        match (self, other) {
            (TyTraitInterfaceItem::TraitFn(id), TyTraitInterfaceItem::TraitFn(other_id)) => {
                id.eq(other_id, engines)
            }
            (TyTraitInterfaceItem::Constant(id), TyTraitInterfaceItem::Constant(other_id)) => {
                id.eq(other_id, engines)
            }
            _ => false,
        }
    }
}

impl EqWithEngines for TyTraitItem {}
impl PartialEqWithEngines for TyTraitItem {
    fn eq(&self, other: &Self, engines: &Engines) -> bool {
        match (self, other) {
            (TyTraitItem::Fn(id), TyTraitItem::Fn(other_id)) => id.eq(other_id, engines),
            (TyTraitItem::Constant(id), TyTraitItem::Constant(other_id)) => {
                id.eq(other_id, engines)
            }
            _ => false,
        }
    }
}

impl HashWithEngines for TyTraitInterfaceItem {
    fn hash<H: Hasher>(&self, state: &mut H, engines: &Engines) {
        match self {
            TyTraitInterfaceItem::TraitFn(fn_decl) => fn_decl.hash(state, engines),
            TyTraitInterfaceItem::Constant(const_decl) => const_decl.hash(state, engines),
            TyTraitInterfaceItem::Type(type_decl) => type_decl.hash(state, engines),
        }
    }
}

impl HashWithEngines for TyTraitItem {
    fn hash<H: Hasher>(&self, state: &mut H, engines: &Engines) {
        match self {
            TyTraitItem::Fn(fn_decl) => fn_decl.hash(state, engines),
            TyTraitItem::Constant(const_decl) => const_decl.hash(state, engines),
            TyTraitItem::Type(type_decl) => type_decl.hash(state, engines),
        }
    }
}

impl TypeCheckFinalization for TyTraitItem {
    fn type_check_finalize(
        &mut self,
        handler: &Handler,
        ctx: &mut TypeCheckFinalizationContext,
    ) -> Result<(), ErrorEmitted> {
        let decl_engine = ctx.engines.de();
        match self {
            TyTraitItem::Fn(node) => {
                let mut item_fn = decl_engine.get_function(node);
                item_fn.type_check_finalize(handler, ctx)?;
                decl_engine.replace(*node.id(), item_fn);
            }
            TyTraitItem::Constant(node) => {
                let mut item_const = decl_engine.get_constant(node);
                item_const.type_check_finalize(handler, ctx)?;
                decl_engine.replace(*node.id(), item_const);
            }
            TyTraitItem::Type(_node) => {
                // Nothing to finalize
            }
        }
        Ok(())
    }
}

impl Spanned for TyTraitItem {
    fn span(&self) -> Span {
        match self {
            TyTraitItem::Fn(fn_decl) => fn_decl.span(),
            TyTraitItem::Constant(const_decl) => const_decl.span(),
            TyTraitItem::Type(type_decl) => type_decl.span(),
        }
    }
}

impl SubstTypes for TyTraitDecl {
    fn subst_inner(&mut self, type_mapping: &TypeSubstMap, engines: &Engines) {
        let mut decl_mapping = DeclMapping::new();
        self.type_parameters
            .iter_mut()
            .for_each(|x| x.subst(type_mapping, engines));
        self.interface_surface
            .iter_mut()
            .for_each(|item| match item {
                TyTraitInterfaceItem::TraitFn(item_ref) => {
                    let new_item_ref = item_ref
                        .clone()
                        .subst_types_and_insert_new_with_parent(type_mapping, engines);
                    decl_mapping.insert(item_ref.id().into(), new_item_ref.id().into());
                    item_ref.replace_id(*new_item_ref.id());
                }
                TyTraitInterfaceItem::Constant(decl_ref) => {
                    let new_decl_ref = decl_ref
                        .clone()
                        .subst_types_and_insert_new(type_mapping, engines);
                    decl_mapping.insert(decl_ref.id().into(), new_decl_ref.id().into());
                    decl_ref.replace_id(*new_decl_ref.id());
                }
                TyTraitInterfaceItem::Type(decl_ref) => {
                    let new_decl_ref = decl_ref
                        .clone()
                        .subst_types_and_insert_new(type_mapping, engines);
                    decl_ref.replace_id(*new_decl_ref.id());
                }
            });
        self.items.iter_mut().for_each(|item| match item {
            TyTraitItem::Fn(item_ref) => {
                let new_item_ref = item_ref
                    .clone()
                    .subst_types_and_insert_new_with_parent(type_mapping, engines);
                item_ref.replace_id(*new_item_ref.id());
            }
            TyTraitItem::Constant(item_ref) => {
                let new_decl_ref = item_ref
                    .clone()
                    .subst_types_and_insert_new_with_parent(type_mapping, engines);
                item_ref.replace_id(*new_decl_ref.id());
            }
            TyTraitItem::Type(item_ref) => {
                let new_decl_ref = item_ref
                    .clone()
                    .subst_types_and_insert_new_with_parent(type_mapping, engines);
                item_ref.replace_id(*new_decl_ref.id());
            }
        });
    }
}

impl SubstTypes for TyTraitItem {
    fn subst_inner(&mut self, type_mapping: &TypeSubstMap, engines: &Engines) {
        match self {
            TyTraitItem::Fn(fn_decl) => fn_decl.subst(type_mapping, engines),
            TyTraitItem::Constant(const_decl) => const_decl.subst(type_mapping, engines),
            TyTraitItem::Type(type_decl) => type_decl.subst(type_mapping, engines),
        }
    }
}

impl ReplaceFunctionImplementingType for TyTraitItem {
    fn replace_implementing_type(&mut self, engines: &Engines, implementing_type: TyDecl) {
        match self {
            TyTraitItem::Fn(decl_ref) => {
                decl_ref.replace_implementing_type(engines, implementing_type)
            }
            TyTraitItem::Constant(_decl_ref) => {
                // ignore, only needed for functions
            }
            TyTraitItem::Type(_decl_ref) => {
                // ignore, only needed for functions
            }
        }
    }
}

impl MonomorphizeHelper for TyTraitDecl {
    fn name(&self) -> &Ident {
        &self.name
    }

    fn type_parameters(&self) -> &[TypeParameter] {
        &self.type_parameters
    }

    fn has_self_type_param(&self) -> bool {
        true
    }
}
