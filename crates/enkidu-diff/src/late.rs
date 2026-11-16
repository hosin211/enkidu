//! enkidu: an element that is built when it is laid out. The view builds its rows that way (the list asks for each as it
//! lays out), after the state it was rendered from is free again; the header that stays in view needs the same, since
//! building a header reads that state and calls the application's own header content.

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    Window,
};

pub(crate) struct Late<F> {
    build: Option<F>,
}

impl<F> Late<F>
where
    F: FnOnce(&mut Window, &mut App) -> AnyElement + 'static,
{
    pub(crate) fn new(build: F) -> Self {
        Self { build: Some(build) }
    }
}

impl<F> Element for Late<F>
where
    F: FnOnce(&mut Window, &mut App) -> AnyElement + 'static,
{
    type RequestLayoutState = Option<AnyElement>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Option<AnyElement>) {
        let mut element = self.build.take().map(|build| build(window, cx));
        let layout = match element.as_mut() {
            Some(element) => element.request_layout(window, cx),
            // laid out a second time: nothing left to build
            None => window.request_layout(gpui::Style::default(), None, cx),
        };
        (layout, element)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        element: &mut Option<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(element) = element {
            element.prepaint(window, cx);
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        element: &mut Option<AnyElement>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(element) = element {
            element.paint(window, cx);
        }
    }
}

impl<F> IntoElement for Late<F>
where
    F: FnOnce(&mut Window, &mut App) -> AnyElement + 'static,
{
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}
