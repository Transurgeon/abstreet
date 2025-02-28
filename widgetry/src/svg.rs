use lyon::math::Point;
use lyon::path::Path;
use lyon::tessellation;
use lyon::tessellation::geometry_builder::{simple_builder, VertexBuffers};

use abstutil::VecMap;
use geom::{Bounds, Pt2D, Tessellation};

use crate::{Color, Fill, GeomBatch, LinearGradient, Prerender};

pub const HIGH_QUALITY: f32 = 0.01;
pub const LOW_QUALITY: f32 = 1.0;

// Code here adapted from
// https://github.com/nical/lyon/blob/0d0ee771180fb317b986d9cf30266722e0773e01/examples/wgpu_svg/src/main.rs

pub fn load_svg(prerender: &Prerender, filename: &str) -> (GeomBatch, Bounds) {
    let cache_key = format!("file://{}", filename);
    if let Some(pair) = prerender.assets.get_cached_svg(&cache_key) {
        return pair;
    }

    let bytes = (prerender.assets.read_svg)(filename);
    load_svg_from_bytes_uncached(&bytes)
        .map(|(batch, bounds)| {
            prerender.assets.cache_svg(cache_key, batch.clone(), bounds);
            (batch, bounds)
        })
        .unwrap_or_else(|_| panic!("error loading svg: {}", filename))
}

pub fn load_svg_bytes(
    prerender: &Prerender,
    cache_key: &str,
    bytes: &[u8],
) -> anyhow::Result<(GeomBatch, Bounds)> {
    let cache_key = format!("bytes://{}", cache_key);
    if let Some(pair) = prerender.assets.get_cached_svg(&cache_key) {
        return Ok(pair);
    }

    load_svg_from_bytes_uncached(bytes).map(|(batch, bounds)| {
        prerender.assets.cache_svg(cache_key, batch.clone(), bounds);
        (batch, bounds)
    })
}

pub fn load_svg_from_bytes_uncached(bytes: &[u8]) -> anyhow::Result<(GeomBatch, Bounds)> {
    let svg_tree = usvg::Tree::from_data(bytes, &usvg::Options::default().to_ref())?;
    let mut batch = GeomBatch::new();
    match add_svg_inner(&mut batch, svg_tree, HIGH_QUALITY) {
        Ok(bounds) => Ok((batch, bounds)),
        Err(err) => Err(anyhow!(err)),
    }
}

// No offset. I'm not exactly sure how the simplification in usvg works, but this doesn't support
// transforms or strokes or text, just fills. Luckily, all of the files exported from Figma so far
// work just fine.
pub(crate) fn add_svg_inner(
    batch: &mut GeomBatch,
    svg_tree: usvg::Tree,
    tolerance: f32,
) -> Result<Bounds, String> {
    let mut fill_tess = tessellation::FillTessellator::new();
    let mut stroke_tess = tessellation::StrokeTessellator::new();
    // TODO This breaks on start.svg; the order there matters. color1, color2, then color1 again.
    let mut mesh_per_color: VecMap<Fill, VertexBuffers<_, u16>> = VecMap::new();

    for node in svg_tree.root().descendants() {
        if let usvg::NodeKind::Path(ref p) = *node.borrow() {
            // TODO Handle transforms

            if let Some(ref fill) = p.fill {
                let color = convert_color(&fill.paint, fill.opacity.value(), &svg_tree);
                let geom = mesh_per_color.mut_or_insert(color, VertexBuffers::new);
                if fill_tess
                    .tessellate(
                        &convert_path(p),
                        &tessellation::FillOptions::tolerance(tolerance),
                        &mut simple_builder(geom),
                    )
                    .is_err()
                {
                    return Err("Couldn't tessellate something".to_string());
                }
            }

            if let Some(ref stroke) = p.stroke {
                let (color, stroke_opts) = convert_stroke(stroke, tolerance, &svg_tree);
                let geom = mesh_per_color.mut_or_insert(color, VertexBuffers::new);
                stroke_tess
                    .tessellate(&convert_path(p), &stroke_opts, &mut simple_builder(geom))
                    .unwrap();
            }
        }
    }

    for (color, mesh) in mesh_per_color.consume() {
        batch.push(
            color,
            Tessellation::new(
                mesh.vertices
                    .into_iter()
                    .map(|v| Pt2D::new(f64::from(v.x), f64::from(v.y)))
                    .collect(),
                mesh.indices.into_iter().map(|idx| idx as usize).collect(),
            ),
        );
    }
    let size = svg_tree.svg_node().size;
    Ok(Bounds::from(&[
        Pt2D::new(0.0, 0.0),
        Pt2D::new(size.width(), size.height()),
    ]))
}

fn convert_path(p: &usvg::Path) -> Path {
    let mut builder = Path::builder().with_svg();
    for segment in p.data.iter() {
        match *segment {
            usvg::PathSegment::MoveTo { x, y } => {
                builder.move_to(Point::new(x as f32, y as f32));
            }
            usvg::PathSegment::LineTo { x, y } => {
                builder.line_to(Point::new(x as f32, y as f32));
            }
            usvg::PathSegment::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                builder.cubic_bezier_to(
                    Point::new(x1 as f32, y1 as f32),
                    Point::new(x2 as f32, y2 as f32),
                    Point::new(x as f32, y as f32),
                );
            }
            usvg::PathSegment::ClosePath => {
                builder.close();
            }
        }
    }
    builder.build()
}

fn convert_stroke(
    s: &usvg::Stroke,
    tolerance: f32,
    tree: &usvg::Tree,
) -> (Fill, tessellation::StrokeOptions) {
    let color = convert_color(&s.paint, s.opacity.value(), tree);
    let linecap = match s.linecap {
        usvg::LineCap::Butt => tessellation::LineCap::Butt,
        usvg::LineCap::Square => tessellation::LineCap::Square,
        usvg::LineCap::Round => tessellation::LineCap::Round,
    };
    let linejoin = match s.linejoin {
        usvg::LineJoin::Miter => tessellation::LineJoin::Miter,
        usvg::LineJoin::Bevel => tessellation::LineJoin::Bevel,
        usvg::LineJoin::Round => tessellation::LineJoin::Round,
    };

    let opt = tessellation::StrokeOptions::tolerance(tolerance)
        .with_line_width(s.width.value() as f32)
        .with_line_cap(linecap)
        .with_line_join(linejoin);

    (color, opt)
}

fn convert_color(paint: &usvg::Paint, opacity: f64, tree: &usvg::Tree) -> Fill {
    match paint {
        usvg::Paint::Color(c) => Fill::Color(Color::rgba(
            c.red as usize,
            c.green as usize,
            c.blue as usize,
            opacity as f32,
        )),
        usvg::Paint::Link(name) => match *tree.defs_by_id(name).unwrap().borrow() {
            usvg::NodeKind::LinearGradient(ref lg) => LinearGradient::new_fill(lg),
            _ => panic!("Unsupported color style {}", name),
        },
    }
}
