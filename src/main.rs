const KERNEL: &[u8] = include_bytes!(env!("compute.spv"));

use glam::{UVec2, Vec4};
use gpgpu::{BufOps, DescriptorSet, Framework, GpuBuffer, GpuBufferUsage, Kernel, Program, Shader};
use std::rc::Rc;
use std::time::Instant;

struct Config {
    width: u32,
    height: u32,
    samples: u32,
}

struct RayGenKernel<'fw> {
    ray_origin_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    ray_dir_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    throughput_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    rng_buffer: Rc<GpuBuffer<'fw, UVec2>>,
    kernel: Kernel<'fw>,
}

impl<'fw> RayGenKernel<'fw> {
    fn new(
        fw: &'fw Framework,
        ray_origin_buffer: Rc<GpuBuffer<'fw, Vec4>>,
        ray_dir_buffer: Rc<GpuBuffer<'fw, Vec4>>,
        throughput_buffer: Rc<GpuBuffer<'fw, Vec4>>,
        rng_buffer: Rc<GpuBuffer<'fw, UVec2>>,
    ) -> Self {
        let shader = Shader::from_spirv_bytes(&fw, KERNEL, Some("compute"));
        let bindings = DescriptorSet::default()
            .bind_buffer(&ray_origin_buffer, GpuBufferUsage::ReadWrite)
            .bind_buffer(&ray_dir_buffer, GpuBufferUsage::ReadWrite)
            .bind_buffer(&throughput_buffer, GpuBufferUsage::ReadWrite)
            .bind_buffer(&rng_buffer, GpuBufferUsage::ReadOnly);
        let program = Program::new(&shader, "main_raygen").add_descriptor_set(bindings);
        let kernel = Kernel::new(&fw, program);

        Self {
            ray_origin_buffer,
            ray_dir_buffer,
            throughput_buffer,
            rng_buffer,
            kernel,
        }
    }
}

struct RayTraceKernel<'fw> {
    ray_origin_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    ray_dir_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    kernel: Kernel<'fw>,
}

impl<'fw> RayTraceKernel<'fw> {
    fn new(
        fw: &'fw Framework,
        ray_origin_buffer: Rc<GpuBuffer<'fw, Vec4>>,
        ray_dir_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    ) -> Self {
        let shader = Shader::from_spirv_bytes(&fw, KERNEL, Some("compute"));
        let bindings = DescriptorSet::default()
            .bind_buffer(&ray_origin_buffer, GpuBufferUsage::ReadWrite)
            .bind_buffer(&ray_dir_buffer, GpuBufferUsage::ReadWrite);
        let program = Program::new(&shader, "main_raytrace").add_descriptor_set(bindings);
        let kernel = Kernel::new(&fw, program);

        Self {
            ray_origin_buffer,
            ray_dir_buffer,
            kernel,
        }
    }
}

struct MaterialKernel<'fw> {
    ray_origin_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    ray_dir_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    throughput_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    rng_buffer: Rc<GpuBuffer<'fw, UVec2>>,
    output_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    kernel: Kernel<'fw>,
}

impl<'fw> MaterialKernel<'fw> {
    fn new(
        fw: &'fw Framework,
        ray_origin_buffer: Rc<GpuBuffer<'fw, Vec4>>,
        ray_dir_buffer: Rc<GpuBuffer<'fw, Vec4>>,
        throughput_buffer: Rc<GpuBuffer<'fw, Vec4>>,
        rng_buffer: Rc<GpuBuffer<'fw, UVec2>>,
        output_buffer: Rc<GpuBuffer<'fw, Vec4>>,
    ) -> Self {
        let shader = Shader::from_spirv_bytes(&fw, KERNEL, Some("compute"));
        let bindings = DescriptorSet::default()
            .bind_buffer(&ray_origin_buffer, GpuBufferUsage::ReadWrite)
            .bind_buffer(&ray_dir_buffer, GpuBufferUsage::ReadWrite)
            .bind_buffer(&throughput_buffer, GpuBufferUsage::ReadWrite)
            .bind_buffer(&rng_buffer, GpuBufferUsage::ReadWrite)
            .bind_buffer(&output_buffer, GpuBufferUsage::ReadWrite);
        let program = Program::new(&shader, "main_material").add_descriptor_set(bindings);
        let kernel = Kernel::new(&fw, program);

        Self {
            ray_origin_buffer,
            ray_dir_buffer,
            throughput_buffer,
            rng_buffer,
            output_buffer,
            kernel,
        }
    }
}

#[cfg(feature = "oidn")]
fn denoise(config: &Config, input: &Vec<f32>) -> Vec<f32> {
    use oidn::filter;
    let mut filter_output = vec![0.0f32; input.len()];
    let device = oidn::Device::new();
    oidn::RayTracing::new(&device)
        .srgb(true)
        .image_dimensions(config.width as usize, config.height as usize)
        .filter(&input[..], &mut filter_output[..])
        .expect("Filter config error!");
    filter_output
}

fn main() {
    let config = Config {
        width: 1280,
        height: 720,
        samples: 1,
    };
    let fw = Framework::default();

    let mut rng = rand::thread_rng();
    let mut rng_data = Vec::new();
    for _ in 0..(config.width * config.height) {
        rng_data.push(UVec2::new(
            rand::Rng::gen(&mut rng),
            rand::Rng::gen(&mut rng),
        ));
    }

    let pixel_count = (config.width * config.height) as u64;
    let ray_origin_buffer = Rc::new(GpuBuffer::with_capacity(&fw, pixel_count));
    let ray_dir_buffer = Rc::new(GpuBuffer::with_capacity(&fw, pixel_count));
    let throughput_buffer = Rc::new(GpuBuffer::with_capacity(&fw, pixel_count));
    let rng_buffer = Rc::new(GpuBuffer::from_slice(&fw, &rng_data));
    let output_buffer = Rc::new(GpuBuffer::with_capacity(&fw, pixel_count));

    let rg = RayGenKernel::new(
        &fw,
        ray_origin_buffer.clone(),
        ray_dir_buffer.clone(),
        throughput_buffer.clone(),
        rng_buffer.clone(),
    );
    let rt = RayTraceKernel::new(&fw, ray_origin_buffer.clone(), ray_dir_buffer.clone());
    let mt = MaterialKernel::new(
        &fw,
        ray_origin_buffer.clone(),
        ray_dir_buffer.clone(),
        throughput_buffer.clone(),
        rng_buffer.clone(),
        output_buffer.clone(),
    );

    let samples = 128;
    let bounces = 4;

    let start = Instant::now();
    for _ in 0..samples {
        rg.kernel.enqueue(config.width / 64, config.height, 1);
        for _ in 0..bounces {
            rt.kernel.enqueue(config.width / 64, config.height, 1);
            mt.kernel.enqueue(config.width / 64, config.height, 1);
        }
    }
    let elapsed = start.elapsed();
    println!("Elapsed: {}ms", elapsed.as_millis());

    let mut image_buffer: Vec<f32> = mt
        .output_buffer
        .read_vec_blocking()
        .unwrap()
        .iter()
        .map(|&x| x / (samples as f32))
        .flat_map(|x| vec![x.x, x.y, x.z])
        .collect();

    #[cfg(feature = "oidn")]
    {
        image_buffer = denoise(&config, &image_buffer);
    }

    let image = image::ImageBuffer::from_fn(config.width, config.height, |x, y| {
        let index = (y * config.width + x) as usize;
        let r = image_buffer[index * 3 + 0];
        let g = image_buffer[index * 3 + 1];
        let b = image_buffer[index * 3 + 2];
        image::Rgb([(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8])
    });
    image.save("image.png").unwrap();
}
