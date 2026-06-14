// Minimal ambient declaration for pixelmatch@6 (ships no types, no @types/* for v6).
declare module "pixelmatch" {
  interface Options {
    threshold?: number;
    includeAA?: boolean;
    alpha?: number;
  }
  export default function pixelmatch(
    img1: Uint8Array | Uint8ClampedArray,
    img2: Uint8Array | Uint8ClampedArray,
    output: Uint8Array | Uint8ClampedArray | null,
    width: number,
    height: number,
    options?: Options,
  ): number;
}
