// The suite's component: its button runs a command that always fails. It
// draws nothing about the failure: oxplow shows it under the frame.
(function () {
  "use strict";
  oxplow.connect().then((component) => {
    component.applyTheme();
    document.getElementById("boom").addEventListener("click", () => {
      component.invoke("e2e.fail.now", {}).catch(() => {});
    });
  });
})();
