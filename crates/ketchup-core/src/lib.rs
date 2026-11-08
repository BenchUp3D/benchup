#[macro_use]
mod i18n;

pub mod connections;
pub mod joinery;
mod cleanup;
pub mod annotations;
pub mod camera;
mod csg;
pub mod display;
pub mod document;
pub mod edit_context;
pub mod faces;
mod history;
pub mod guides;
pub mod inference;
pub mod measurements;
pub mod modeling;
pub mod objects;
mod reparent;
pub mod selection;
pub mod solids;
pub mod surface_treatment;
pub mod tags;
pub mod units;

pub use annotations::TextAnnotation;
pub use guides::ConstructionGuide;
pub use camera::{Camera, CameraView, Projection, Ray, StandardView};
pub use display::{DisplaySettings, FaceStyle};
pub use document::{Document, Edge, Face, History, Model, Vertex};
pub use edit_context::InstanceTransform;
pub use glam::{DVec2, DVec3};
pub use inference::{
    Axis, AxisGuide, Inference, MoveInference, Snap, infer, infer_move_with_snapping,
    infer_with_snapping,
};
pub use measurements::{Dimension, DimensionAnchor, angle_arc, angle_degrees};
pub use objects::{Object, ObjectKind, StockMaterial};
pub use solids::SolidOperation;
pub use tags::Tag;
pub use units::{AngleUnit, AreaUnit, LengthUnit, UnitPrecision, UnitSettings, VolumeUnit};

#[cfg(test)]
mod annotation_tests;
#[cfg(test)]
mod edit_context_tests;
#[cfg(test)]
mod measurement_tests;
#[cfg(test)]
mod modeling_tests;
#[cfg(test)]
mod move_inference_tests;
#[cfg(test)]
mod object_tests;
#[cfg(test)]
mod reference_inference_tests;
#[cfg(test)]
mod reparent_tests;
#[cfg(test)]
mod solid_tests;
#[cfg(test)]
mod surface_tests;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    #[default]
    Select,
    Line,
    Rectangle,
    Circle,
    Polygon,
    Freehand,
    PushPull,
    Flip,
    OuterShell,
    Union,
    Intersect,
    Trim,
    Split,
    Scale,
    Offset,
    Move,
    Rotate,
    Paint,
    TapeMeasure,
    Protractor,
    Dimensions,
    Text,
    Erase,
    Orbit,
}

impl Tool {
    pub const SOLID: [Self; 6] = [
        Self::Flip,
        Self::OuterShell,
        Self::Union,
        Self::Intersect,
        Self::Trim,
        Self::Split,
    ];
    pub fn is_solid(self) -> bool {
        Self::SOLID.contains(&self)
    }

    pub const MEASURING: [Self; 4] = [
        Self::Dimensions,
        Self::TapeMeasure,
        Self::Protractor,
        Self::Text,
    ];
    pub fn is_measuring(self) -> bool {
        Self::MEASURING.contains(&self)
    }
    pub const DRAWING: [Self; 5] = [
        Self::Line,
        Self::Rectangle,
        Self::Circle,
        Self::Polygon,
        Self::Freehand,
    ];
    pub fn is_drawing(self) -> bool {
        Self::DRAWING.contains(&self)
    }
    pub const ALL: [Self; 24] = [
        Self::Select,
        Self::Line,
        Self::Rectangle,
        Self::Circle,
        Self::Polygon,
        Self::Freehand,
        Self::PushPull,
        Self::Flip,
        Self::OuterShell,
        Self::Union,
        Self::Intersect,
        Self::Trim,
        Self::Split,
        Self::Scale,
        Self::Offset,
        Self::Move,
        Self::Rotate,
        Self::Paint,
        Self::TapeMeasure,
        Self::Protractor,
        Self::Text,
        Self::Dimensions,
        Self::Erase,
        Self::Orbit,
    ];
    pub const RAIL: [Self; 12] = [
        Self::Select,
        Self::Line,
        Self::Erase,
        Self::Orbit,
        Self::PushPull,
        Self::Scale,
        Self::Offset,
        Self::Move,
        Self::Rotate,
        Self::Paint,
        Self::Dimensions,
        Self::Union,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => tr!("tools.select"),
            Self::Line => tr!("tools.line"),
            Self::Rectangle => tr!("tools.rectangle"),
            Self::Circle => tr!("tools.circle"),
            Self::Polygon => tr!("tools.polygon"),
            Self::Freehand => tr!("tools.freehand"),
            Self::PushPull => tr!("tools.push_pull"),
            Self::Flip => tr!("tools.flip"),
            Self::OuterShell => tr!("tools.combine"),
            Self::Union => tr!("tools.union"),
            Self::Intersect => tr!("tools.intersect"),
            Self::Trim => tr!("tools.trim"),
            Self::Split => tr!("tools.split"),
            Self::Scale => tr!("tools.scale"),
            Self::Offset => tr!("tools.offset"),
            Self::Move => tr!("tools.move"),
            Self::Rotate => tr!("tools.rotate"),
            Self::Paint => tr!("tools.paint"),
            Self::TapeMeasure => tr!("tools.tape_measure"),
            Self::Protractor => tr!("tools.protractor"),
            Self::Dimensions => tr!("tools.dimensions"),
            Self::Text => tr!("tools.text"),
            Self::Erase => tr!("tools.erase"),
            Self::Orbit => tr!("tools.orbit"),
        }
    }
    pub fn shortcut(self) -> &'static str {
        match self {
            Self::Select => "Space",
            Self::Line => "L",
            Self::Rectangle => "R",
            Self::Circle => "C",
            Self::Polygon => "G",
            Self::Freehand => "D",
            Self::PushPull => "P",
            Self::Flip
            | Self::OuterShell
            | Self::Union
            | Self::Intersect
            | Self::Trim
            | Self::Split => "",
            Self::Scale => "S",
            Self::Offset => "F",
            Self::Move => "M",
            Self::Rotate => "Q",
            Self::Paint => "B",
            Self::TapeMeasure => "T",
            Self::Protractor => "A",
            Self::Dimensions => "I",
            Self::Text => "",
            Self::Erase => "E",
            Self::Orbit => "O",
        }
    }
    pub fn instructions(self) -> &'static str {
        match self {
            Self::Select => tr!("tools.click_an_edge_or_face_drag_to_box"),
            Self::Line => {
                tr!("tools.click_to_start_a_line_arrow_keys_lock")
            }
            Self::Rectangle => tr!("tools.click_two_opposite_corners_arrow_keys_lock_the"),
            Self::Circle => {
                tr!("tools.click_the_center_then_a_point_on_the")
            }
            Self::Polygon => tr!("tools.click_the_center_then_a_vertex_arrow_keys"),
            Self::Freehand => {
                tr!("tools.drag_to_draw_release_to_finish_arrow_keys")
            }
            Self::Erase => tr!("tools.click_or_drag_across_edges_or_faces_to"),
            Self::Flip => {
                tr!("tools.select_geometry_choose_a_red_green_or_blue")
            }
            Self::OuterShell => tr!("tools.click_two_closed_solids_to_combine_their_outer"),
            Self::Union => tr!("tools.click_two_closed_solids_to_combine_their_volumes"),
            Self::Intersect => tr!("tools.click_groups_components_or_faces_to_keep_their"),
            Self::Trim => tr!("tools.click_the_cutter_first_then_the_solid_to"),
            Self::Split => {
                tr!("tools.click_two_closed_solids_to_keep_the_overlap")
            }
            Self::PushPull => {
                tr!("tools.click_a_face_move_along_its_normal_arrow")
            }
            Self::Scale => {
                tr!("tools.select_geometry_click_a_reference_point_move_or")
            }
            Self::Offset => {
                tr!("tools.click_a_face_near_its_boundary_move_inward")
            }
            Self::Move => {
                tr!("tools.select_geometry_click_a_base_point_and_destination")
            }
            Self::Rotate => {
                tr!("tools.select_geometry_click_a_pivot_reference_point_then")
            }
            Self::Paint => {
                tr!("tools.choose_a_color_and_face_or_group_in")
            }
            Self::TapeMeasure => tr!("tools.click_a_reference_edge_or_point_ctrl_command"),
            Self::Protractor => {
                tr!("tools.click_a_center_a_baseline_then_an_angle")
            }
            Self::Dimensions => {
                tr!("tools.click_two_endpoints_then_click_to_place_the")
            }
            Self::Text => tr!("tools.click_an_object_then_click_to_place_its"),
            Self::Orbit => tr!("tools.drag_to_orbit_hold_shift_to_pan_scroll"),
        }
    }
}

/// Stable identity shared by catalog entries and document material assignments.
pub type MaterialId = uuid::Uuid;

pub use document::SharedProject;

pub use history::{HistoryAuthor, HistoryAction};

#[cfg(test)]
mod face_center_tests;
