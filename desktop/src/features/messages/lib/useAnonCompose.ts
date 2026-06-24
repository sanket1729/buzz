import * as React from "react";

export type AnonComposeState = {
  /** True once a ring is chosen and anonymous mode is armed. */
  armed: boolean;
  /** Canonical (sorted, deduped, lowercase) ring including the signer. */
  ring: string[];
  pickerOpen: boolean;
  openPicker: () => void;
  closePicker: () => void;
  /** Arm with the confirmed canonical ring and close the picker. */
  confirmRing: (ring: string[]) => void;
  /** Turn anonymous mode off and drop the ring. */
  disarm: () => void;
  /** Reset after a send — anonymous mode is never sticky (UX §3.2). */
  reset: () => void;
};

/**
 * Local compose-side state for anonymous send: whether the next message is
 * armed anonymous, the chosen ring, and the picker's open state. Resets on
 * channel change so anonymous mode never silently carries across channels.
 * Methods are stable so callers can memoize against this state.
 */
export function useAnonCompose(channelId: string | null): AnonComposeState {
  const [armed, setArmed] = React.useState(false);
  const [ring, setRing] = React.useState<string[]>([]);
  const [pickerOpen, setPickerOpen] = React.useState(false);

  // Reset when the channel changes. channelId is the trigger, not a value the
  // body reads, so it reads as an "extra" dependency to the linter.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on channel change
  React.useEffect(() => {
    setArmed(false);
    setRing([]);
    setPickerOpen(false);
  }, [channelId]);

  const openPicker = React.useCallback(() => setPickerOpen(true), []);
  const closePicker = React.useCallback(() => setPickerOpen(false), []);
  const confirmRing = React.useCallback((next: string[]) => {
    setRing(next);
    setArmed(true);
    setPickerOpen(false);
  }, []);
  const disarm = React.useCallback(() => {
    setArmed(false);
    setRing([]);
  }, []);
  const reset = React.useCallback(() => {
    setArmed(false);
    setRing([]);
    setPickerOpen(false);
  }, []);

  return React.useMemo(
    () => ({
      armed,
      ring,
      pickerOpen,
      openPicker,
      closePicker,
      confirmRing,
      disarm,
      reset,
    }),
    [
      armed,
      ring,
      pickerOpen,
      openPicker,
      closePicker,
      confirmRing,
      disarm,
      reset,
    ],
  );
}
