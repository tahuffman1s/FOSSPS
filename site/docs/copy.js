// Adds a "Copy" button to every command and file on the page. Without
// JavaScript the page reads the same; the text is selected by hand.
(function () {
  if (!navigator.clipboard) return;
  var status = document.createElement("p");
  status.setAttribute("role", "status");
  status.style.cssText = "position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%)";
  document.body.appendChild(status);

  document.querySelectorAll("pre[data-cmd], pre[data-file]").forEach(function (pre) {
    var what = pre.hasAttribute("data-file")
      ? "the file " + pre.getAttribute("data-file")
      : "the command " + pre.textContent.trim().split("\n")[0];
    var button = document.createElement("button");
    button.type = "button";
    button.textContent = "Copy";
    button.setAttribute("aria-label", "Copy " + what);
    button.addEventListener("click", function () {
      navigator.clipboard.writeText(pre.textContent.replace(/\n$/, "")).then(
        function () {
          button.textContent = "Copied";
          status.textContent = "Copied " + what;
          // Emptied afterwards, so copying the same thing again is announced again.
          setTimeout(function () { button.textContent = "Copy"; status.textContent = ""; }, 2000);
        },
        function () {
          status.textContent = "Could not copy. Select the text and copy it by hand.";
        }
      );
    });
    pre.classList.add("has-copy");
    pre.parentNode.appendChild(button);
  });
})();
