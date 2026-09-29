// Shared shapes for the conformance harness.

/** A laid-out box: its `data-cid` tag and border-box rectangle in absolute px
 *  (page/viewport origin at top-left). The engine JSON and the browser's
 *  `getBoundingClientRect()` both map onto this. */
export interface Box {
  cid: string;
  x: number;
  y: number;
  width: number;
  height: number;
}
