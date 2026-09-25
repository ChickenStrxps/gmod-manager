import { loadFont } from "@remotion/google-fonts/PlusJakartaSans";
import React from "react";
import {
  AbsoluteFill,
  Audio,
  Easing,
  OffthreadVideo,
  Sequence,
  interpolate,
  random,
  spring,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import manifest from "./manifest.generated.json";

const { fontFamily } = loadFont("normal", { weights: ["400", "500", "700", "800"] });

// The app's own palette (src/main.rs).
const BG = "#17191d";
const TEXT = "#e2e5e9";
const MUTED = "#8b929d";
const ACCENT = "58, 138, 245";
const AMBER = "224, 166, 74";

type Scene = (typeof manifest.scenes)[number];

const CAPTURE = { width: 1600, height: 1000 };
const CARD_WIDTH = 1180;
const CARD_SCALE = CARD_WIDTH / CAPTURE.width;
const TITLE_BAR = 34;
const FIRST_CAPTURE = manifest.scenes.find((s) => s.kind === "capture")?.id;

const ease = Easing.bezier(0.22, 1, 0.36, 1);

/** Soft glows drifting over the app's background colour, plus grain and a vignette. */
const Backdrop: React.FC = () => {
  const frame = useCurrentFrame();
  const t = frame / manifest.fps;
  const blue = { x: 22 + 8 * Math.sin(t * 0.21), y: 18 + 6 * Math.cos(t * 0.17) };
  const warm = { x: 82 + 6 * Math.cos(t * 0.19), y: 86 + 5 * Math.sin(t * 0.23) };
  return (
    <AbsoluteFill style={{ background: BG }}>
      <AbsoluteFill
        style={{
          background: [
            `radial-gradient(900px circle at ${blue.x}% ${blue.y}%, rgba(${ACCENT}, 0.22), transparent 70%)`,
            `radial-gradient(800px circle at ${warm.x}% ${warm.y}%, rgba(${AMBER}, 0.13), transparent 70%)`,
            `radial-gradient(1400px circle at 50% 120%, rgba(${ACCENT}, 0.08), transparent 70%)`,
          ].join(","),
        }}
      />
      <AbsoluteFill style={{ opacity: 0.07, mixBlendMode: "overlay" }}>
        <svg width="100%" height="100%">
          <filter id="grain">
            <feTurbulence
              type="fractalNoise"
              baseFrequency="0.9"
              numOctaves="2"
              seed={Math.floor(frame / 2) % 60}
            />
          </filter>
          <rect width="100%" height="100%" filter="url(#grain)" />
        </svg>
      </AbsoluteFill>
      <AbsoluteFill
        style={{ background: "radial-gradient(circle at 50% 45%, transparent 55%, rgba(0,0,0,0.55) 100%)" }}
      />
    </AbsoluteFill>
  );
};

/** Words rise and settle one after another. */
const Words: React.FC<{ text: string; delay: number; size: number; color: string; weight: number }> = ({
  text,
  delay,
  size,
  color,
  weight,
}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return (
    <div style={{ fontSize: size, color, fontWeight: weight, lineHeight: 1.15, letterSpacing: size > 60 ? -1.5 : -0.3 }}>
      {text.split(" ").map((word, i) => {
        const p = spring({ frame: frame - delay - i * 3, fps, config: { damping: 18, stiffness: 120 } });
        return (
          <span
            key={i}
            style={{
              display: "inline-block",
              marginRight: size * 0.26,
              opacity: p,
              transform: `translateY(${(1 - p) * size * 0.45}px)`,
              filter: `blur(${(1 - p) * 6}px)`,
            }}
          >
            {word}
          </span>
        );
      })}
    </div>
  );
};

/** The app's mark: a rounded tile with a play arrow, as on the Play button. */
const Mark: React.FC<{ size: number }> = ({ size }) => (
  <div
    style={{
      width: size,
      height: size,
      borderRadius: size * 0.28,
      background: `linear-gradient(145deg, rgb(${ACCENT}), #2a5fc0)`,
      boxShadow: `0 ${size * 0.2}px ${size * 0.6}px rgba(${ACCENT}, 0.45), inset 0 2px 0 rgba(255,255,255,0.25)`,
      display: "flex",
      alignItems: "center",
      justifyContent: "center",
    }}
  >
    <svg width={size * 0.42} height={size * 0.42} viewBox="0 0 10 10">
      <path d="M2.5 1.2 L8.6 5 L2.5 8.8 Z" fill="white" strokeLinejoin="round" stroke="white" strokeWidth="0.8" />
    </svg>
  </div>
);

const TitleScene: React.FC<{ scene: Scene; outro: boolean }> = ({ scene, outro }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const pop = spring({ frame: frame - 4, fps, config: { damping: 12, stiffness: 110 } });
  const breathe = 1 + 0.015 * Math.sin(frame / 18);
  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", gap: 34, flexDirection: "column" }}>
      <div style={{ transform: `scale(${pop * breathe}) rotate(${(1 - pop) * -12}deg)`, opacity: pop }}>
        <Mark size={150} />
      </div>
      <Words text={scene.headline} delay={10} size={116} color={TEXT} weight={800} />
      <Words text={scene.sub} delay={22} size={outro ? 34 : 38} color={MUTED} weight={500} />
      {outro && (
        <div
          style={{
            marginTop: 10,
            opacity: interpolate(frame, [40, 60], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" }),
            padding: "14px 30px",
            borderRadius: 999,
            background: `rgba(${ACCENT}, 0.16)`,
            border: `1px solid rgba(${ACCENT}, 0.45)`,
            color: TEXT,
            fontSize: 30,
            fontWeight: 700,
          }}
        >
          Download it free · Windows
        </div>
      )}
    </AbsoluteFill>
  );
};

/** The recorded app in a window frame, easing in on the first scene and drifting gently. */
const CaptureScene: React.FC<{ scene: Scene }> = ({ scene }) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  const entering = scene.id === FIRST_CAPTURE;
  const rise = entering ? spring({ frame, fps, config: { damping: 20, stiffness: 70 } }) : 1;
  const drift = interpolate(frame, [0, scene.duration], [1, 1.025]);
  const cardHeight = CAPTURE.height * CARD_SCALE + TITLE_BAR;
  const top = 1080 - cardHeight - 80;
  // Push in on the part of the app the narration is about, then ease back out.
  const focus = scene.focus;
  const push = focus
    ? spring({ frame: frame - focus.from, fps, config: { damping: 26, stiffness: 55 } }) *
      (1 - spring({ frame: frame - focus.to, fps, config: { damping: 26, stiffness: 55 } }))
    : 0;
  const zoom = 1 + ((focus?.zoom ?? 1) - 1) * push;
  return (
    <AbsoluteFill>
      <div style={{ position: "absolute", left: 120, top: 58, width: 1680 }}>
        <Words text={scene.headline} delay={4} size={62} color={TEXT} weight={800} />
        <div style={{ height: 10 }} />
        <Words text={scene.sub} delay={12} size={30} color={MUTED} weight={500} />
      </div>
      <div
        style={{
          position: "absolute",
          left: (1920 - CARD_WIDTH) / 2 + 60,
          top,
          width: CARD_WIDTH,
          height: cardHeight,
          perspective: 2200,
        }}
      >
        <div
          style={{
            width: "100%",
            height: "100%",
            transformOrigin: "50% 100%",
            transform: `translateY(${(1 - rise) * 120}px) rotateX(${(1 - rise) * 14}deg) scale(${drift})`,
            opacity: rise,
            borderRadius: 16,
            overflow: "hidden",
            background: "#121417",
            border: "1px solid rgba(255,255,255,0.09)",
            boxShadow: `0 40px 120px rgba(0,0,0,0.6), 0 0 90px rgba(${ACCENT}, 0.16)`,
          }}
        >
          <div
            style={{
              height: TITLE_BAR,
              display: "flex",
              alignItems: "center",
              gap: 10,
              padding: "0 14px",
              background: "#121417",
              borderBottom: "1px solid rgba(255,255,255,0.06)",
              color: MUTED,
              fontSize: 15,
              fontWeight: 600,
            }}
          >
            <Mark size={18} />
            GMod Manager
          </div>
          {scene.clip && (
            <div style={{ overflow: "hidden", width: CARD_WIDTH, height: CAPTURE.height * CARD_SCALE }}>
              <OffthreadVideo
                src={staticFile(scene.clip)}
                muted
                style={{
                  width: CARD_WIDTH,
                  height: CAPTURE.height * CARD_SCALE,
                  display: "block",
                  transformOrigin: `${(focus?.x ?? 0.5) * 100}% ${(focus?.y ?? 0.5) * 100}%`,
                  transform: `scale(${zoom})`,
                }}
              />
            </div>
          )}
        </div>
      </div>
    </AbsoluteFill>
  );
};

/** Fades a scene in and out across the crossfade. */
const Fade: React.FC<{ scene: Scene; children: React.ReactNode; first: boolean; last: boolean }> = ({
  scene,
  children,
  first,
  last,
}) => {
  const frame = useCurrentFrame();
  const xf = manifest.crossfade;
  const fadeIn = first ? 1 : interpolate(frame, [0, xf], [0, 1], { extrapolateRight: "clamp", easing: ease });
  const fadeOut = last
    ? interpolate(frame, [scene.duration - xf * 2, scene.duration], [1, 0], { extrapolateLeft: "clamp" })
    : interpolate(frame, [scene.duration - xf, scene.duration], [1, 0], { extrapolateLeft: "clamp" });
  return <AbsoluteFill style={{ opacity: Math.min(fadeIn, fadeOut) }}>{children}</AbsoluteFill>;
};

const VersionBadge: React.FC = () => {
  const frame = useCurrentFrame();
  const shown = interpolate(frame, [20, 45], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" });
  return (
    <div
      style={{
        position: "absolute",
        right: 40,
        bottom: 34,
        opacity: shown * 0.9,
        transform: `translateY(${(1 - shown) * 10}px)`,
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "9px 18px",
        borderRadius: 999,
        background: "rgba(18,20,23,0.7)",
        border: "1px solid rgba(255,255,255,0.12)",
        color: MUTED,
        fontSize: 20,
        fontWeight: 600,
      }}
    >
      <span style={{ width: 8, height: 8, borderRadius: 4, background: `rgb(${ACCENT})` }} />
      GMod Manager <span style={{ color: TEXT }}>v{manifest.version}</span>
    </div>
  );
};

/** Music sits back while someone is talking. */
const musicVolume = (frame: number) => {
  const ramp = 12;
  let duck = 0;
  for (const scene of manifest.scenes) {
    const start = scene.from + scene.voice.at;
    const end = start + scene.voiceFrames;
    duck = Math.max(
      duck,
      interpolate(frame, [start - ramp, start, end, end + ramp * 2], [0, 1, 1, 0], {
        extrapolateLeft: "clamp",
        extrapolateRight: "clamp",
      }),
    );
  }
  const fadeIn = interpolate(frame, [0, 40], [0, 1], { extrapolateRight: "clamp" });
  const fadeOut = interpolate(frame, [manifest.total - 90, manifest.total], [1, 0], { extrapolateLeft: "clamp" });
  return (0.2 - 0.12 * duck) * Math.min(fadeIn, fadeOut);
};

export const Intro: React.FC = () => {
  const last = manifest.scenes.length - 1;
  return (
    <AbsoluteFill style={{ fontFamily }}>
      <Backdrop />
      <Audio src={staticFile(manifest.music)} volume={musicVolume} />
      {manifest.scenes.map((scene, index) => (
        <Sequence key={scene.id} from={scene.from} durationInFrames={scene.duration} name={scene.id}>
          <Fade scene={scene} first={index === 0} last={index === last}>
            {scene.kind === "title" ? <TitleScene scene={scene} outro={index === last} /> : <CaptureScene scene={scene} />}
          </Fade>
          <Sequence from={scene.voice.at} name={`${scene.id} voice`}>
            <Audio src={staticFile(scene.voice.src)} volume={1} />
          </Sequence>
          {scene.sfx.map((cue, i) => (
            <Sequence key={i} from={Math.max(0, cue.at)} durationInFrames={manifest.fps} name={`${scene.id} sfx`}>
              <Audio
                src={staticFile(cue.src)}
                volume={cue.volume}
                playbackRate={0.94 + random(`${scene.id}-${i}`) * 0.12}
              />
            </Sequence>
          ))}
        </Sequence>
      ))}
      <VersionBadge />
    </AbsoluteFill>
  );
};
