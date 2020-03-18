//! Items related to the inter-operation of the `image` crate (images on disk and in RAM) and
//! textures from the wgpu crate (images in GPU memory).

use crate::wgpu;
use std::path::Path;
use std::slice;

/// The set of pixel types from the image crate that can be loaded directly into a texture.
///
/// The `Rgba8` and `Bgra8` color types are assumed to be non-linear sRGB.
///
/// Note that wgpu only supports texture formats whose size are a power of 2. If you notice a
/// `image::Pixel` type that does not implement `Pixel`, this is likely why.
pub trait Pixel: image::Pixel {
    /// The wgpu texture format of the pixel type.
    const TEXTURE_FORMAT: wgpu::TextureFormat;
}

/// A wrapper around a wgpu buffer that contains an image of a known size and `image::ColorType`.
#[derive(Debug)]
pub struct BufferImage {
    color_type: image::ColorType,
    size: [u32; 2],
    buffer: wgpu::BufferBytes,
}

/// A wrapper around a slice of bytes representing an image.
///
/// An `ImageAsyncMapping` may only be created by reading from a `BufferImage` returned by a
/// `Texture::to_image` call.
pub struct ImageAsyncMapping<'a> {
    color_type: image::ColorType,
    size: [u32; 2],
    mapping: wgpu::BufferAsyncMapping<&'a [u8]>,
}

impl wgpu::TextureBuilder {
    /// Produce a texture descriptor from an image.
    ///
    /// Specifically, this supports any image type implementing `image::GenericImageView` whose
    /// `Pixel` type implements `Pixel`.
    pub fn from_image_view<T>(image_view: &T) -> Self
    where
        T: image::GenericImageView,
        T::Pixel: Pixel,
    {
        builder_from_image_view(image_view)
    }
}

impl wgpu::Texture {
    /// Load a texture directly from an image buffer using the given device queue.
    ///
    /// No format or size conversions are performed - the given buffer is loaded directly into GPU
    /// memory.
    ///
    /// Pixel type compatibility is ensured via the `Pixel` trait.
    pub fn load_from_image_buffer<P, Container>(
        device: &wgpu::Device,
        queue: &mut wgpu::Queue,
        usage: wgpu::TextureUsage,
        buffer: &image::ImageBuffer<P, Container>,
    ) -> wgpu::Texture
    where
        P: 'static + Pixel,
        Container: std::ops::Deref<Target = [P::Subpixel]>,
    {
        load_texture_from_image_buffer(device, queue, usage, buffer)
    }

    /// Load a texture array directly from a sequence of image buffers.
    ///
    /// No format or size conversions are performed - the given buffer is loaded directly into GPU
    /// memory.
    ///
    /// Pixel type compatibility is ensured via the `Pixel` trait.
    ///
    /// Returns `None` if there are no images in the given sequence.
    pub fn load_array_from_image_buffers<'a, I, P, Container>(
        device: &wgpu::Device,
        queue: &mut wgpu::Queue,
        usage: wgpu::TextureUsage,
        buffers: I,
    ) -> Option<Self>
    where
        I: IntoIterator<Item = &'a image::ImageBuffer<P, Container>>,
        I::IntoIter: ExactSizeIterator,
        P: 'static + Pixel,
        Container: 'a + std::ops::Deref<Target = [P::Subpixel]>,
    {
        load_texture_array_from_image_buffers(device, queue, usage, buffers)
    }

    /// Encode the necessary commands to load a texture from the given image buffer.
    ///
    /// NOTE: The returned texture will remain empty until the given `encoder` has its command
    /// buffer submitted to the given `device`'s queue.
    ///
    /// No format or size conversions are performed - the given buffer is loaded directly into GPU
    /// memory.
    ///
    /// Pixel type compatibility is ensured via the `Pixel` trait.
    pub fn encode_load_from_image_buffer<P, Container>(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        usage: wgpu::TextureUsage,
        buffer: &image::ImageBuffer<P, Container>,
    ) -> Self
    where
        P: 'static + Pixel,
        Container: std::ops::Deref<Target = [P::Subpixel]>,
    {
        encode_load_texture_from_image_buffer(device, encoder, usage, buffer)
    }

    /// Encode the necessary commands to load a texture array directly from a sequence of image
    /// buffers.
    ///
    /// NOTE: The returned texture will remain empty until the given `encoder` has its command buffer
    /// submitted to the given `device`'s queue.
    ///
    /// No format or size conversions are performed - the given buffer is loaded directly into GPU
    /// memory.
    ///
    /// Pixel type compatibility is ensured via the `Pixel` trait.
    ///
    /// Returns `None` if there are no images in the given sequence.
    pub fn encode_load_array_from_image_buffers<'a, I, P, Container>(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        usage: wgpu::TextureUsage,
        buffers: I,
    ) -> Option<Self>
    where
        I: IntoIterator<Item = &'a image::ImageBuffer<P, Container>>,
        I::IntoIter: ExactSizeIterator,
        P: 'static + Pixel,
        Container: 'a + std::ops::Deref<Target = [P::Subpixel]>,
    {
        encode_load_texture_array_from_image_buffers(device, encoder, usage, buffers)
    }

    /// Write the contents of the texture into a new image buffer.
    ///
    /// Commands will be added to the given encoder to copy the entire contents of the texture into
    /// the buffer.
    ///
    /// Returns a buffer from which the image can be read asynchronously via `read`.
    ///
    /// Returns `None` if there is no directly compatible `image::ColorType` for the texture's format.
    ///
    /// NOTE: `read` should not be called on the returned buffer until the encoded commands have
    /// been submitted to the device queue.
    pub fn to_image(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<BufferImage> {
        let color_type = image_color_type_from_format(self.format())?;
        let size = self.size();
        let buffer = self.to_buffer_bytes(device, encoder);
        Some(BufferImage {
            color_type,
            size,
            buffer,
        })
    }
}

impl BufferImage {
    /// The dimensions of the image stored within the buffer.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// The color type of the image stored within the buffer.
    pub fn color_type(&self) -> image::ColorType {
        self.color_type
    }

    /// Asynchronously maps the buffer of bytes from GPU to host memory and, once mapped, calls the
    /// given user callback with the data represented as an `ImageAsyncMapping`.
    ///
    /// Note: The given callback will not be called until the memory is mapped and the device is
    /// polled. You should not rely on the callback being called immediately.
    pub fn read<F>(&self, callback: F)
    where
        F: 'static + FnOnce(Result<ImageAsyncMapping, ()>),
    {
        let size = self.size;
        let color_type = self.color_type;
        self.buffer.read(move |result| {
            let result = result.map(|mapping| ImageAsyncMapping {
                color_type,
                size,
                mapping,
            });
            callback(result);
        })
    }
}

impl<'a> ImageAsyncMapping<'a> {
    /// Produce the color type of an image, compatible with the `image` crate.
    pub fn color_type(&self) -> image::ColorType {
        self.color_type
    }

    /// The dimensions of the image.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// The raw image data as a slice of bytes.
    pub fn mapping(&self) -> &wgpu::BufferAsyncMapping<&[u8]> {
        &self.mapping
    }

    /// Saves the buffer to a file at the specified path.
    ///
    /// The image format is derived from the file extension.
    pub fn save(&self, path: &Path) -> image::ImageResult<()> {
        let [width, height] = self.size();
        image::save_buffer(path, &self.mapping.data, width, height, self.color_type)
    }

    /// Saves the buffer to a file at the specified path.
    pub fn save_with_format(
        &self,
        path: &Path,
        format: image::ImageFormat,
    ) -> image::ImageResult<()> {
        let [width, height] = self.size();
        image::save_buffer_with_format(
            path,
            &self.mapping.data,
            width,
            height,
            self.color_type,
            format,
        )
    }

    /// Attempt to cast this image ref to an `ImageBuffer` of the specified pixel type.
    ///
    /// Returns `None` if the specified pixel type does not match the inner `color_type`.
    pub fn as_image_buffer<P>(&self) -> Option<image::ImageBuffer<P, &[P::Subpixel]>>
    where
        P: 'static + Pixel,
    {
        if P::COLOR_TYPE != self.color_type {
            return None;
        }
        let [width, height] = self.size();
        let len_pixels = (width * height) as usize;
        let subpixel_data_ptr = self.mapping.data.as_ptr() as *const _;
        let subpixel_data: &[P::Subpixel] =
            unsafe { slice::from_raw_parts(subpixel_data_ptr, len_pixels) };
        let img_buffer = image::ImageBuffer::from_raw(width, height, subpixel_data)
            .expect("failed to construct image buffer from raw data");
        Some(img_buffer)
    }
}

impl Pixel for image::Bgra<u8> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;
}

impl Pixel for image::Luma<u8> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
}

impl Pixel for image::Luma<i8> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Snorm;
}

impl Pixel for image::Luma<u16> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Unorm;
}

impl Pixel for image::Luma<i16> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Snorm;
}

impl Pixel for image::LumaA<u8> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
}

impl Pixel for image::LumaA<i8> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Snorm;
}

impl Pixel for image::LumaA<u16> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Unorm;
}

impl Pixel for image::LumaA<i16> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Snorm;
}

impl Pixel for image::Rgba<u8> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
}

impl Pixel for image::Rgba<i8> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Snorm;
}

impl Pixel for image::Rgba<u16> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Unorm;
}

impl Pixel for image::Rgba<i16> {
    const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Snorm;
}

/// Convert the given color type from the `image` crate to the corresponding wgpu texture format.
///
/// Returns `None` if there is no directly compatible texture format - this is normally the case if
/// the `ColorType` would have a bits_per_pixel that is not equal to a power of 2.
///
/// The `Rgba8` and `Bgra8` color types are assumed to be non-linear sRGB.
pub fn format_from_image_color_type(color_type: image::ColorType) -> Option<wgpu::TextureFormat> {
    let format = match color_type {
        image::ColorType::L8 => wgpu::TextureFormat::R8Unorm,
        image::ColorType::La8 => wgpu::TextureFormat::Rg8Unorm,
        image::ColorType::Rgba8 => wgpu::TextureFormat::Rgba8UnormSrgb,
        image::ColorType::L16 => wgpu::TextureFormat::R16Unorm,
        image::ColorType::La16 => wgpu::TextureFormat::Rg16Unorm,
        image::ColorType::Rgba16 => wgpu::TextureFormat::Rgba16Unorm,
        image::ColorType::Bgra8 => wgpu::TextureFormat::Bgra8UnormSrgb,
        _ => return None,
    };
    Some(format)
}

/// Convert the given texture format to the corresponding color type from the `image` crate.
///
/// Returns `None` if there is no directly compatible color type.
///
/// The `Rgba8` and `Bgra8` color types are assumed to be non-linear sRGB.
pub fn image_color_type_from_format(format: wgpu::TextureFormat) -> Option<image::ColorType> {
    let color_type = match format {
        // TODO: Should we add branches for other same-size formats? e.g. R8Snorm, R8Uint, etc?
        wgpu::TextureFormat::R8Unorm => image::ColorType::L8,
        wgpu::TextureFormat::Rg8Unorm => image::ColorType::La8,
        wgpu::TextureFormat::Rgba8UnormSrgb => image::ColorType::Rgba8,
        wgpu::TextureFormat::R16Unorm => image::ColorType::L16,
        wgpu::TextureFormat::Rg16Unorm => image::ColorType::La16,
        wgpu::TextureFormat::Rgba16Unorm => image::ColorType::Rgba16,
        wgpu::TextureFormat::Bgra8UnormSrgb => image::ColorType::Bgra8,
        _ => return None,
    };
    Some(color_type)
}

/// Produce a texture descriptor from any type implementing `image::GenericImageView` whose `Pixel`
/// type implements `Pixel`.
///
/// This function does not specify a texture usage.
pub fn builder_from_image_view<T>(image: &T) -> wgpu::TextureBuilder
where
    T: image::GenericImageView,
    T::Pixel: Pixel,
{
    let (width, height) = image.dimensions();
    let format = <T::Pixel as Pixel>::TEXTURE_FORMAT;
    wgpu::TextureBuilder::new()
        .size([width, height])
        .format(format)
}

/// Load a texture directly from an image buffer using the given device queue.
///
/// No format or size conversions are performed - the given buffer is loaded directly into GPU
/// memory.
///
/// Pixel type compatibility is ensured via the `Pixel` trait.
pub fn load_texture_from_image_buffer<P, Container>(
    device: &wgpu::Device,
    queue: &mut wgpu::Queue,
    usage: wgpu::TextureUsage,
    buffer: &image::ImageBuffer<P, Container>,
) -> wgpu::Texture
where
    P: 'static + Pixel,
    Container: std::ops::Deref<Target = [P::Subpixel]>,
{
    let cmd_encoder_desc = wgpu::CommandEncoderDescriptor::default();
    let mut encoder = device.create_command_encoder(&cmd_encoder_desc);
    let texture = encode_load_texture_from_image_buffer(device, &mut encoder, usage, buffer);
    queue.submit(&[encoder.finish()]);
    texture
}

/// Encode the necessary commands to load a texture directly from an image buffer.
///
/// NOTE: The returned texture will remain empty until the given `encoder` has its command buffer
/// submitted to the given `device`'s queue.
///
/// No format or size conversions are performed - the given buffer is loaded directly into GPU
/// memory.
///
/// Pixel type compatibility is ensured via the `Pixel` trait.
pub fn encode_load_texture_from_image_buffer<P, Container>(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    usage: wgpu::TextureUsage,
    buffer: &image::ImageBuffer<P, Container>,
) -> wgpu::Texture
where
    P: 'static + Pixel,
    Container: std::ops::Deref<Target = [P::Subpixel]>,
{
    // Create the texture.
    let texture = wgpu::TextureBuilder::from_image_view(buffer)
        .usage(wgpu::TextureUsage::COPY_DST | usage)
        .build(device);

    // Upload the pixel data.
    let subpixel_data: &[P::Subpixel] = std::ops::Deref::deref(buffer);
    let buffer = device
        .create_buffer_mapped(subpixel_data.len(), wgpu::BufferUsage::COPY_SRC)
        .fill_from_slice(subpixel_data);

    // Submit command for copying pixel data to the texture.
    let buffer_copy_view = texture.default_buffer_copy_view(&buffer);
    let texture_copy_view = texture.default_copy_view();
    let extent = texture.extent();
    encoder.copy_buffer_to_texture(buffer_copy_view, texture_copy_view, extent);

    texture
}

/// Load a texture array directly from a sequence of image buffers.
///
/// No format or size conversions are performed - the given buffer is loaded directly into GPU
/// memory.
///
/// Pixel type compatibility is ensured via the `Pixel` trait.
///
/// Returns `None` if there are no images in the given sequence.
pub fn load_texture_array_from_image_buffers<'a, I, P, Container>(
    device: &wgpu::Device,
    queue: &mut wgpu::Queue,
    usage: wgpu::TextureUsage,
    buffers: I,
) -> Option<wgpu::Texture>
where
    I: IntoIterator<Item = &'a image::ImageBuffer<P, Container>>,
    I::IntoIter: ExactSizeIterator,
    P: 'static + Pixel,
    Container: 'a + std::ops::Deref<Target = [P::Subpixel]>,
{
    let cmd_encoder_desc = wgpu::CommandEncoderDescriptor::default();
    let mut encoder = device.create_command_encoder(&cmd_encoder_desc);
    let texture =
        encode_load_texture_array_from_image_buffers(device, &mut encoder, usage, buffers);
    queue.submit(&[encoder.finish()]);
    texture
}

/// Encode the necessary commands to load a texture array directly from a sequence of image
/// buffers.
///
/// NOTE: The returned texture will remain empty until the given `encoder` has its command buffer
/// submitted to the given `device`'s queue.
///
/// No format or size conversions are performed - the given buffer is loaded directly into GPU
/// memory.
///
/// Pixel type compatibility is ensured via the `Pixel` trait.
///
/// Returns `None` if there are no images in the given sequence.
pub fn encode_load_texture_array_from_image_buffers<'a, I, P, Container>(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    usage: wgpu::TextureUsage,
    buffers: I,
) -> Option<wgpu::Texture>
where
    I: IntoIterator<Item = &'a image::ImageBuffer<P, Container>>,
    I::IntoIter: ExactSizeIterator,
    P: 'static + Pixel,
    Container: 'a + std::ops::Deref<Target = [P::Subpixel]>,
{
    let mut buffers = buffers.into_iter();
    let array_layers = buffers.len() as u32;
    let first_buffer = buffers.next()?;

    // Build the texture ready to receive the data.
    let texture = wgpu::TextureBuilder::from_image_view(first_buffer)
        .array_layer_count(array_layers)
        .usage(wgpu::TextureUsage::COPY_DST | usage)
        .build(device);

    // Copy each buffer to the texture, one layer at a time.
    for (layer, buffer) in Some(first_buffer).into_iter().chain(buffers).enumerate() {
        // Upload the pixel data.
        let subpixel_data: &[P::Subpixel] = std::ops::Deref::deref(buffer);
        let buffer = device
            .create_buffer_mapped(subpixel_data.len(), wgpu::BufferUsage::COPY_SRC)
            .fill_from_slice(subpixel_data);

        // Submit command for copying pixel data to the texture.
        let buffer_copy_view = texture.default_buffer_copy_view(&buffer);
        let mut texture_copy_view = texture.default_copy_view();
        texture_copy_view.array_layer = layer as u32;
        let extent = texture.extent();
        encoder.copy_buffer_to_texture(buffer_copy_view, texture_copy_view, extent);
    }

    Some(texture)
}
