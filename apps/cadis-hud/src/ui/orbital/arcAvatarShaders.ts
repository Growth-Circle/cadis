export const avatarVertexShader = /* glsl */ `
  varying vec2 vUv;
  uniform float uTime;
  uniform float uAmplitude;

  void main() {
    vUv = uv;
    vec3 p = position;
    float wave = sin((uv.y * 8.0) + uTime * 1.2) * 0.006;
    p.z += wave * (0.35 + uAmplitude);
    gl_Position = projectionMatrix * modelViewMatrix * vec4(p, 1.0);
  }
`;

export const avatarFragmentShader = /* glsl */ `
  varying vec2 vUv;

  uniform sampler2D uTexture;
  uniform vec3 uModeColor;
  uniform float uTime;
  uniform float uOpacity;
  uniform float uAmplitude;
  uniform float uBackgroundCutoff;
  uniform float uGlow;

  float luma(vec3 color) {
    return dot(color, vec3(0.299, 0.587, 0.114));
  }

  void main() {
    vec4 tex = texture2D(uTexture, vUv);
    float brightness = luma(tex.rgb);
    float bgMask = smoothstep(uBackgroundCutoff, uBackgroundCutoff + 0.09, brightness);
    float scan = 0.72 + 0.28 * sin((vUv.y * 132.0) - (uTime * 2.0));
    vec2 centered = vUv - vec2(0.5);
    float radius = length(centered);
    float vignette = 1.0 - smoothstep(0.60, 0.86, radius);

    vec3 base = pow(tex.rgb, vec3(0.82));
    vec3 cyberTint = base * (1.12 + uGlow * 0.34 + uAmplitude * 0.18);
    cyberTint = mix(cyberTint, cyberTint + uModeColor * (0.18 + scan * 0.055), 0.34);
    cyberTint += uModeColor * vignette * 0.035;

    float alpha = tex.a * bgMask * (0.82 + vignette * 0.18) * uOpacity;
    if (alpha < 0.01) discard;

    gl_FragColor = vec4(cyberTint, alpha);
  }
`;
