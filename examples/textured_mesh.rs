use nannou::prelude::*;

fn main() {
    nannou::app(model).run();
}

struct Model {
    window_id: window::Id,
    texture_view: wgpu::TextureView,
}

fn model(app: &App) -> Model {
    let window_id = app.new_window().size(512, 512).view(view).build().unwrap();
    let window = app.window(window_id).unwrap();

    // Load the image from disk and upload it to a GPU texture.
    let img_path = app
        .assets_path()
        .unwrap()
        .join("images")
        .join("nature")
        .join("nature_1.jpg");
    let image = image::open(img_path).unwrap();
    let image_rgba = image.into_rgba();
    let usage = wgpu::TextureUsage::SAMPLED;
    let device = window.swap_chain_device();
    let mut queue = window.swap_chain_queue().lock().unwrap();
    let texture = wgpu::Texture::load_from_image_buffer(device, &mut *queue, usage, &image_rgba);
    let texture_view = texture.view().build();

    Model {
        window_id,
        texture_view,
    }
}

// Draw the state of your `Model` into the given `Frame` here.
fn view(app: &App, model: &Model, frame: Frame) {
    frame.clear(DIMGRAY);
    let window = app.window(model.window_id).unwrap();
    let win_rect = window.rect();
    let draw = app.draw();

    // Generate the triangulated points for a cuboid to use for out mesh.
    let centre = pt3(0.0, 0.0, 0.0);
    let size = vec3(1.0, 1.0, 1.0);
    let cuboid = geom::Cuboid::from_xyz_whd(centre, size);
    let points = cuboid
        .triangles_iter()
        .flat_map(geom::Tri::vertices)
        .map(|point| {
            // Tex coords should be in range (0.0, 0.0) to (1.0, 1.0);
            // This will have the logo show on the front and back faces.
            let tex_coords = [point.x + 0.5, 1.0 - (point.y + 0.5)];
            (point, tex_coords)
        });

    // Scale the points up to half the window size.
    let cube_side = win_rect.w().min(win_rect.h()) * 0.5;
    draw.scale(cube_side)
        .mesh()
        .points_textured(&model.texture_view, points)
        .z_radians(app.time * 0.33)
        .x_radians(app.time * 0.166 + -app.mouse.y / 100.0)
        .y_radians(app.time * 0.25 + app.mouse.x / 100.0);

    // Draw to the frame!
    draw.to_frame(app, &frame).unwrap();
}
