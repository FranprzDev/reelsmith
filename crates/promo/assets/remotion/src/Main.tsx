import React, {useEffect, useState} from 'react';
import {
  AbsoluteFill,
  Audio,
  OffthreadVideo,
  Sequence,
  continueRender,
  delayRender,
  interpolate,
  spring,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from 'remotion';

export type PlanProps = {
  durationInFrames: number;
  fps: number;
  width: number;
  height: number;
  shots: {src: string; from: number; dur: number; startFrom: number; z0: number; z1: number}[];
  captions: {text: string; from: number; dur: number; size: number}[];
  caption: {bottom: number; fill: string; outline: string; uppercase: boolean};
  flashes: {from: number; dur: number; opacity: number}[];
  audio: string | null;
};

const Shot: React.FC<{shot: PlanProps['shots'][number]}> = ({shot}) => {
  const frame = useCurrentFrame();
  const scale = interpolate(frame, [0, shot.dur], [shot.z0, shot.z1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  return (
    <AbsoluteFill>
      <OffthreadVideo
        src={staticFile(shot.src)}
        startFrom={shot.startFrom}
        endAt={shot.startFrom + shot.dur}
        muted
        style={{width: '100%', height: '100%', objectFit: 'cover', transform: `scale(${scale})`}}
      />
    </AbsoluteFill>
  );
};

const Caption: React.FC<{caption: PlanProps['captions'][number]; look: PlanProps['caption']}> = ({caption, look}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const enter = spring({frame, fps, config: {damping: 14, stiffness: 160, mass: 0.6}});
  const exit = interpolate(frame, [caption.dur - 3, caption.dur], [1, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  return (
    <div
      style={{
        position: 'absolute',
        left: 0,
        right: 0,
        bottom: look.bottom,
        textAlign: 'center',
        fontFamily: "'ReelsmithCaption', sans-serif",
        fontSize: caption.size,
        lineHeight: 1.05,
        padding: '0 12px',
        color: look.fill,
        textTransform: look.uppercase ? 'uppercase' : 'none',
        textShadow: `0 2px 0 ${look.outline}, 0 0 18px rgba(0,0,0,.85)`,
        opacity: Math.min(enter, exit),
        transform: `translateY(${(1 - enter) * 14}px)`,
      }}
    >
      {caption.text}
    </div>
  );
};

const Flash: React.FC<{flash: PlanProps['flashes'][number]}> = ({flash}) => {
  const frame = useCurrentFrame();
  const opacity = interpolate(frame, [0, flash.dur], [flash.opacity, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  return <AbsoluteFill style={{backgroundColor: '#fff', opacity}} />;
};

export const Main: React.FC<PlanProps> = (props) => {
  const [handle] = useState(() => delayRender());
  useEffect(() => {
    const font = new FontFace('ReelsmithCaption', `url('${staticFile('assets/font.ttf')}')`);
    font
      .load()
      .then((loaded) => {
        document.fonts.add(loaded);
        continueRender(handle);
      })
      .catch(() => continueRender(handle));
  }, [handle]);
  return (
    <AbsoluteFill style={{backgroundColor: '#000'}}>
      {props.shots.map((shot, i) => (
        <Sequence key={i} from={shot.from} durationInFrames={shot.dur}>
          <Shot shot={shot} />
        </Sequence>
      ))}
      {props.captions.map((caption, i) => (
        <Sequence key={i} from={caption.from} durationInFrames={caption.dur}>
          <Caption caption={caption} look={props.caption} />
        </Sequence>
      ))}
      {props.flashes.map((flash, i) => (
        <Sequence key={i} from={flash.from} durationInFrames={flash.dur}>
          <Flash flash={flash} />
        </Sequence>
      ))}
      {props.audio ? <Audio src={staticFile(props.audio)} /> : null}
    </AbsoluteFill>
  );
};
