/*
 * Copyright (c) 2010 Arc90 Inc
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
var REGEXPS = {
    unlikelyCandidates: /-ad-|ai2html|banner|breadcrumbs|combx|comment|community|cover-wrap|disqus|extra|footer|gdpr|header|legends|menu|related|remark|replies|rss|shoutbox|sidebar|skyscraper|social|sponsor|supplemental|ad-break|agegate|pagination|pager|popup|yom-remote/i,
    okMaybeItsACandidate: /and|article|body|column|content|main|shadow/i
};

function isNodeVisible(node) {
    return (!node.style || node.style.display != "none") && !node.hasAttribute("hidden") && (!node.hasAttribute("aria-hidden") || node.getAttribute("aria-hidden") != "true" || node.className && node.className.includes && node.className.includes("fallback-image"));
}

function isProbablyReaderable(doc, options = {}) {
    if (typeof options == "function") {
        options = {
            visibilityChecker: options
        };
    }
    var defaultOptions = {
        minScore: 20,
        minContentLength: 140,
        visibilityChecker: isNodeVisible
    };
    options = Object.assign(defaultOptions, options);
    var nodes = doc.querySelectorAll("p, pre, article");
    var brNodes = doc.querySelectorAll("div > br");
    if (brNodes.length) {
        var set = new Set(nodes);
        [].forEach.call(brNodes, (function(node) {
            set.add(node.parentNode);
        }));
        nodes = Array.from(set);
    }
    var score = 0;
    return [].some.call(nodes, (function(node) {
        if (!options.visibilityChecker(node)) {
            return false;
        }
        var matchString = node.className + " " + node.id;
        if (REGEXPS.unlikelyCandidates.test(matchString) && !REGEXPS.okMaybeItsACandidate.test(matchString)) {
            return false;
        }
        if (node.matches("li p")) {
            return false;
        }
        var textContentLength = node.textContent.trim().length;
        if (textContentLength < options.minContentLength) {
            return false;
        }
        score += Math.sqrt(textContentLength - options.minContentLength);
        if (score > options.minScore) {
            return true;
        }
        return false;
    }));
}
