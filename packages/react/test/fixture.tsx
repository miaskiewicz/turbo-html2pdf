// A representative document exercising several control-flow + paged-media +
// plain-HTML features at once. Used by the byte-equality test (AC-8.7) and shared
// conceptually with the second frontend's equivalent (AC-8.9). Modeled on the
// invoice in spec Appendix A.1.

import { createElement as h, Fragment } from "react";
import {
  Case,
  Default,
  Each,
  Else,
  ElseIf,
  Expr,
  Footnote,
  If,
  Include,
  Leader,
  Page,
  Pages,
  Region,
  RunningFooter,
  Running,
  Switch,
} from "../src/index.js";

export function InvoiceDoc() {
  return (
    <Fragment>
      <RunningFooter extent="12mm">
        Page <Page /> of <Pages />
        <Leader />
        <span>{"<flux & co>"}</span>
      </RunningFooter>
      <Running name="section" as="h1">
        <Expr value="invoice.title" />
      </Running>
      <If cond='invoice.status == "paid"'>
        <p className="muted">Paid in full.</p>
        <ElseIf cond='invoice.status == "overdue"'>
          <p>
            Overdue by <Expr value="invoice.days_overdue" /> days.
          </p>
        </ElseIf>
        <Else>
          <p>Due soon.</p>
        </Else>
      </If>
      <table className="lines">
        <tbody>
          <Each of="invoice.lines" as="line">
            <tr>
              <td>
                <Expr value="loop.index" />
              </td>
              <td>
                <Expr value="line.description" />
                <If cond="line.note">
                  <Footnote>
                    <Expr value="line.note" />
                  </Footnote>
                </If>
              </td>
              <td className="right">
                <Expr value="line.amount | currency(invoice.ccy)" />
              </td>
            </tr>
            <Else>
              {h("tr", null, h("td", { colSpan: 3, className: "muted" }, "No line items."))}
            </Else>
          </Each>
        </tbody>
      </table>
      <Switch on="customer.tier">
        <Case value='"enterprise"'>
          <p>Dedicated support included.</p>
        </Case>
        <Case value='"pro", "plus"'>
          <p>Priority support included.</p>
        </Case>
        <Default>
          <p>Standard support.</p>
        </Default>
      </Switch>
      <Region slot="footer">
        <Expr value="doc.confidential" />
      </Region>
      <Include src='"remittance"' with="company.bank" />
    </Fragment>
  );
}
