use super::*;
mod view_image;
pub(super) fn register(r: &mut Registry, inference: Arc<dyn Inference>) {
    r.register(
        "view_image",
        Category::Vision,
        Authority::WorkspaceRead,
        Arc::new(view_image::ViewImage { inference }),
    );
}
