// (common.wgsl is put in front)

struct Mat {
    color: vec4<f32>,
    // alpha mode (0 opaque, 1 test, 2 blend), has texture, two-sided lighting, 0
    mode: vec4<f32>,
    // normal map (1, 2 = GL style), occlusion, roughness, metalness present
    pbr: vec4<f32>,
};

@group(1) @binding(0) var<uniform> mat: Mat;
@group(1) @binding(1) var albedo: texture_2d<f32>;
@group(1) @binding(2) var normal_map: texture_2d<f32>;
@group(1) @binding(3) var orm_map: texture_2d<f32>;
@group(1) @binding(4) var samp: sampler;
// mode.w: 1 a night texture (glows by night), 2 a light map (lights the texture by night)
@group(1) @binding(5) var glow_map: texture_2d<f32>;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // instance: model matrix (world, relative to nothing) and a tint
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    @location(7) tint: vec4<f32>,
    // x: surface code (1 a spline, 1.25 an object lying on the road, 0 anything else)
    @location(8) params: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) rel: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tint: vec4<f32>,
};

@vertex
fn vs(v: VsIn) -> VsOut {
    var o: VsOut;
    let m = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    // (the translation is taken apart so the origin is subtracted before precision is lost)
    let lin = mat3x3<f32>(v.m0.xyz, v.m1.xyz, v.m2.xyz);
    let rel = lin * v.pos + (v.m3.xyz - g.origin.xyz);
    o.rel = rel;
    // Road surfaces are pulled towards the eye along the line of sight - the picture does
    // not move, only the depth - so they win over the flush ground beside them (as
    // openOMSI's vs_main does: 0.3 % of the way, a little more for objects on the road).
    var cp = rel;
    let surf = v.params.x;
    if (surf > 0.9) {
        let to = rel - g.eye.xyz;
        let d = length(to);
        let decal = select(0.0, 0.01 + 0.001 * d, surf > 1.1 && surf < 1.5);
        let pull = min(0.003 * d + decal, d * 0.3);
        cp = rel - to / max(d, 1e-3) * pull;
    }
    o.clip = g.view_proj * vec4<f32>(cp, 1.0);
    o.normal = normalize(lin * v.normal);
    o.uv = v.uv;
    o.tint = v.tint;
    return o;
}

// A tangent frame from the screen-space derivatives: no tangents needed in the mesh
// (as openOMSI's enhanced shader does it).
fn perturb(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>, t: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let tt = dp2perp * duv1.x + dp1perp * duv2.x;
    let bb = dp2perp * duv1.y + dp1perp * duv2.y;
    let invmax = inverseSqrt(max(dot(tt, tt), dot(bb, bb)) + 1e-12);
    let tbn = mat3x3<f32>(tt * invmax, bb * invmax, n);
    return normalize(tbn * t);
}

@fragment
fn fs(i: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let nm = textureSample(normal_map, samp, i.uv);
    let orm = textureSample(orm_map, samp, i.uv);
    var base = mat.color;
    if (mat.mode.y > 0.5) {
        let t = textureSample(albedo, samp, i.uv);
        base = vec4<f32>(t.rgb, t.a * mat.color.a);
        if (mat.mode.x < 0.5) {
            base.a = 1.0;
        }
    }
    if (mat.mode.x > 0.5 && mat.mode.x < 1.5 && base.a < 0.5) {
        discard;
    }
    var n = normalize(i.normal);
    if (!front) {
        n = -n;
    }
    if (mat.pbr.x > 0.5) {
        var t = nm.xyz * 2.0 - 1.0;
        // Direct3D style maps have green pointing down
        if (mat.pbr.x < 1.5) {
            t.y = -t.y;
        }
        n = perturb(n, i.rel, i.uv, normalize(t));
    }
    var ao = 1.0;
    var rough = 0.75;
    var metal = 0.0;
    if (mat.pbr.y > 0.5) { ao = orm.r; }
    if (mat.pbr.z > 0.5) { rough = orm.g; }
    if (mat.pbr.w > 0.5) { metal = orm.b; }
    let v = normalize(g.eye.xyz - i.rel);
    var col = shade(base.rgb, n, v, rough, metal, ao, sun_light(i.rel, normalize(i.normal) * select(-1.0, 1.0, front)));
    let glow = textureSample(glow_map, samp, i.uv);
    if (mat.mode.w > 0.5 && g.shadow.z > 0.0) {
        if (mat.mode.w < 1.5) {
            col += glow.rgb * g.shadow.z * 2.0;
        } else {
            col += base.rgb * glow.rgb * g.shadow.z * 2.0;
        }
    }
    col = tonemap(apply_fog(col, i.rel));
    if (i.tint.a < 0.0) {
        // the selection's glow: orange light at the rim and a little all over.
        // abs(alpha) scales it, so a spline can wear a lighter wash than an object
        let rim = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 1.1);
        col = mix(col, i.tint.rgb, (0.14 + 0.36 * rim) * abs(i.tint.a));
    } else {
        col = mix(col, i.tint.rgb, i.tint.a);
    }
    return vec4<f32>(col, base.a);
}

// The shadow pass: depth only, cut-out texels (leaves, fences) let the sun through.
@fragment
fn fs_shadow(i: VsOut) {
    if (mat.mode.x > 0.5 && mat.mode.y > 0.5) {
        let a = textureSample(albedo, samp, i.uv).a * mat.color.a;
        if (a < 0.5) {
            discard;
        }
    }
}
