import React from 'react';
import {Composition} from 'remotion';
import {Main, PlanProps} from './Main';

export const Root: React.FC = () => {
  return (
    <Composition
      id="Plan"
      component={Main}
      durationInFrames={1}
      fps={30}
      width={1080}
      height={1920}
      defaultProps={{} as unknown as PlanProps}
      calculateMetadata={({props}) => ({
        durationInFrames: props.durationInFrames,
        fps: props.fps,
        width: props.width,
        height: props.height,
      })}
    />
  );
};
