use crate::{data, render, settings};
use render::{
    BindingInfo, Buffer, Context, DoubleFramebuffer, Framebuffer, Indices, Program, RenderPass,
    TextureOptions, Uniform, UniformValue, VertexBuffer,
};
use settings::Noise;

use web_sys::WebGl2RenderingContext as GL;

static FLUID_VERT_SHADER: &'static str = include_str!("./shaders/fluid.vert");
static SIMPLEX_NOISE_FRAG_SHADER: &'static str = include_str!("./shaders/simplex_noise.frag");
static BLEND_WITH_CURL: &'static str = include_str!("./shaders/blend_with_curl.frag");
static BLEND_WITH_WIGGLE: &'static str = include_str!("./shaders/blend_with_wiggle.frag");

pub struct NoiseChannel {
    noise: Noise,
    texture: Framebuffer,
    blend_begin_time: f32,
    last_blend_progress: f32,
    offset1: f32,
    offset2: f32,
}

impl NoiseChannel {
    pub fn generate(&mut self, generate_noise_pass: &RenderPass, elapsed_time: f32) -> () {
        let width = self.texture.width;
        let height = self.texture.height;

        generate_noise_pass
            .draw_to(
                &self.texture,
                &vec![
                    Uniform {
                        name: "uResolution",
                        value: UniformValue::Vec2([width as f32, height as f32]),
                    },
                    Uniform {
                        name: "uOffset1",
                        value: UniformValue::Float(self.offset1),
                    },
                    Uniform {
                        name: "uOffset2",
                        value: UniformValue::Float(self.offset2),
                    },
                    Uniform {
                        name: "uOffsetIncrement",
                        value: UniformValue::Float(self.noise.offset_increment),
                    },
                    Uniform {
                        name: "uFrequency",
                        value: UniformValue::Float(self.noise.scale),
                    },
                ],
                1,
            )
            .unwrap();

        self.blend_begin_time = elapsed_time;
        self.last_blend_progress = 0.0;
        self.offset1 += self.noise.offset_increment;
        self.offset2 += self.noise.offset_increment;
    }
}

pub struct NoiseInjector {
    context: Context,
    pub channels: Vec<NoiseChannel>,
    width: u32,
    height: u32,
    generate_noise_pass: RenderPass,
    blend_with_curl_pass: RenderPass,
    blend_with_wiggle_pass: RenderPass,
}

impl NoiseInjector {
    pub fn update_channel(&mut self, channel_number: usize, new_noise: Noise) -> () {
        if let Some(channel) = self.channels.get_mut(channel_number) {
            channel.noise = new_noise.clone();
        }
    }

    pub fn new(context: &Context, width: u32, height: u32) -> Result<Self, render::Problem> {
        // Geometry
        let plane_vertices = Buffer::from_f32(
            &context,
            &data::PLANE_VERTICES.to_vec(),
            GL::ARRAY_BUFFER,
            GL::STATIC_DRAW,
        )?;
        let plane_indices = Buffer::from_u16(
            &context,
            &data::PLANE_INDICES.to_vec(),
            GL::ELEMENT_ARRAY_BUFFER,
            GL::STATIC_DRAW,
        )?;

        let simplex_noise_program =
            Program::new(&context, (FLUID_VERT_SHADER, SIMPLEX_NOISE_FRAG_SHADER))?;
        let blend_with_curl_program = Program::new(&context, (FLUID_VERT_SHADER, BLEND_WITH_CURL))?;
        let blend_with_wiggle_program =
            Program::new(&context, (FLUID_VERT_SHADER, BLEND_WITH_WIGGLE))?;

        let generate_noise_pass = RenderPass::new(
            &context,
            vec![VertexBuffer {
                buffer: plane_vertices.clone(),
                binding: BindingInfo {
                    name: "position".to_string(),
                    size: 3,
                    type_: GL::FLOAT,
                    ..Default::default()
                },
            }],
            Indices::IndexBuffer {
                buffer: plane_indices.clone(),
                primitive: GL::TRIANGLES,
            },
            simplex_noise_program,
        )?;

        let blend_with_curl_pass = RenderPass::new(
            &context,
            vec![VertexBuffer {
                buffer: plane_vertices.clone(),
                binding: BindingInfo {
                    name: "position".to_string(),
                    size: 3,
                    type_: GL::FLOAT,
                    ..Default::default()
                },
            }],
            Indices::IndexBuffer {
                buffer: plane_indices.clone(),
                primitive: GL::TRIANGLES,
            },
            blend_with_curl_program,
        )?;

        let blend_with_wiggle_pass = RenderPass::new(
            &context,
            vec![VertexBuffer {
                buffer: plane_vertices.clone(),
                binding: BindingInfo {
                    name: "position".to_string(),
                    size: 3,
                    type_: GL::FLOAT,
                    ..Default::default()
                },
            }],
            Indices::IndexBuffer {
                buffer: plane_indices.clone(),
                primitive: GL::TRIANGLES,
            },
            blend_with_wiggle_program,
        )?;

        Ok(Self {
            context: context.clone(),
            channels: Vec::new(),
            width,
            height,
            generate_noise_pass,
            blend_with_curl_pass,
            blend_with_wiggle_pass,
        })
    }

    pub fn add_noise(&mut self, noise: Noise) -> Result<(), render::Problem> {
        let texture = Framebuffer::new(
            &self.context,
            self.width,
            self.height,
            TextureOptions {
                mag_filter: GL::LINEAR,
                min_filter: GL::LINEAR,
                format: GL::RG32F,
                ..Default::default()
            },
        )?
        .with_f32_data(&vec![0.0; (self.width * self.height * 2) as usize])?;

        self.channels.push(NoiseChannel {
            noise: noise.clone(),
            texture,
            blend_begin_time: 0.0,
            last_blend_progress: 0.0,
            offset1: noise.offset_1,
            offset2: noise.offset_2,
        });

        Ok(())
    }

    pub fn generate_all(&mut self, elapsed_time: f32) -> () {
        for channel in self.channels.iter_mut() {
            let time_since_last_update = elapsed_time - channel.blend_begin_time;

            if time_since_last_update >= channel.noise.delay {
                channel.generate(&self.generate_noise_pass, elapsed_time);
            }
        }
    }

    pub fn generate_by_channel_number(&mut self, channel_number: usize, elapsed_time: f32) {
        if let Some(channel) = self.channels.get_mut(channel_number) {
            channel.generate(&self.generate_noise_pass, elapsed_time);
        }
    }

    pub fn blend_noise_into(&mut self, textures: &DoubleFramebuffer, elapsed_time: f32) -> () {
        for channel in self.channels.iter_mut() {
            let blend_progress: f32 = ((elapsed_time - channel.blend_begin_time)
                / channel.noise.blend_duration)
                .clamp(0.0, 1.0);

            if blend_progress >= 1.0 - 0.0001 {
                continue;
            }

            let delta_blend_progress = blend_progress - channel.last_blend_progress;
            let blend_pass: &RenderPass = match channel.noise.blend_method {
                settings::BlendMethod::Curl => &self.blend_with_curl_pass,
                settings::BlendMethod::Wiggle => &self.blend_with_wiggle_pass,
            };

            blend_pass
                .draw_to(
                    &textures.next(),
                    &vec![
                        Uniform {
                            name: "uTexelSize",
                            value: UniformValue::Vec2([
                                1.0 / self.width as f32,
                                1.0 / self.height as f32,
                            ]),
                        },
                        Uniform {
                            name: "uMultiplier",
                            value: UniformValue::Float(channel.noise.multiplier),
                        },
                        Uniform {
                            name: "uBlendProgress",
                            value: UniformValue::Float(delta_blend_progress),
                        },
                        Uniform {
                            name: "inputTexture",
                            value: UniformValue::Texture2D(&textures.current().texture, 0),
                        },
                        Uniform {
                            name: "noiseTexture",
                            value: UniformValue::Texture2D(&channel.texture.texture, 1),
                        },
                    ],
                    1,
                )
                .unwrap();

            textures.swap();
            channel.last_blend_progress = blend_progress;
        }
    }

    #[allow(dead_code)]
    pub fn get_noise_channel(&self, channel_number: usize) -> Option<&Framebuffer> {
        self.channels
            .get(channel_number)
            .map(|channel| &channel.texture)
    }
}
