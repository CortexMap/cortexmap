import { serve } from "bun";

const ORCH_BASE = "https://capstone.ssdd.dev/orch";

const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, PATCH, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

function json(data: unknown, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json", ...CORS },
  });
}

async function proxyGet(path: string) {
  const res = await fetch(`${ORCH_BASE}${path}`);
  const data = await res.json();
  return json(data, res.status);
}

serve({
  port: 3001,
  async fetch(req) {
    const url = new URL(req.url);
    const path = url.pathname;

    // Handle CORS preflight
    if (req.method === "OPTIONS") {
      return new Response(null, { headers: CORS });
    }

    // Serve static frontend
    if (path === "/" || path === "/index.html") {
      const file = Bun.file("./index.html");
      return new Response(file, {
        headers: { "Content-Type": "text/html" },
      });
    }

    // Serve 3D brain viewer page
    if (path === "/brain3d.html") {
      const file = Bun.file("./brain3d.html");
      return new Response(file, {
        headers: { "Content-Type": "text/html" },
      });
    }

    // Serve CSS
    if (path === "/styles.css") {
      const file = Bun.file("./styles.css");
      return new Response(file, {
        headers: { "Content-Type": "text/css" },
      });
    }

    // Serve JavaScript files
    if (path === "/app.js" || path === "/brain3d.js") {
      const file = Bun.file(`.${path}`);
      return new Response(file, {
        headers: { "Content-Type": "application/javascript" },
      });
    }

    // Proxy BrainBrowser library to add CORS headers
    if (path === "/js/brainbrowser.min.js") {
      console.log("📥 Proxying BrainBrowser library...");
      try {
        const response = await fetch("https://brainbrowser.cbrain.mcgill.ca/js/brainbrowser-2.5.2.min.js");
        const content = await response.text();
        return new Response(content, {
          headers: {
            "Content-Type": "application/javascript",
            "Access-Control-Allow-Origin": "*",
            "Cache-Control": "public, max-age=86400", // Cache for 1 day
          },
        });
      } catch (error) {
        console.error("❌ Failed to proxy BrainBrowser:", error);
        return new Response("// BrainBrowser library could not be loaded", {
          status: 500,
          headers: { "Content-Type": "application/javascript" },
        });
      }
    }
      // Proxy BrainBrowser data files
    const BRAINBROWSER_DATA: Record<string, string> = {
      "/brainbrowser/spectral.txt": "https://brainbrowser.cbrain.mcgill.ca/color-maps/spectral.txt",
      "/brainbrowser/brain-surface.obj": "https://brainbrowser.cbrain.mcgill.ca/models/brain-surface.obj",
      "/brainbrowser/dti.txt": "https://brainbrowser.cbrain.mcgill.ca/models/dti.txt",
    };

    if (path in BRAINBROWSER_DATA) {
      console.log(`📥 Proxying BrainBrowser data: ${path}`);
      try {
        const response = await fetch(BRAINBROWSER_DATA[path]!);
        const content = await response.arrayBuffer();
        return new Response(content, {
          headers: {
            "Content-Type": response.headers.get("content-type") ?? "text/plain",
            "Access-Control-Allow-Origin": "*",
            "Cache-Control": "public, max-age=86400",
          },
       });
      } catch (error) {
        console.error(`❌ Failed to proxy ${path}:`, error);
        return new Response("Not found", { status: 500 });
      }
    }
    

    // Serve static assets
    if (path.startsWith("/assets/")) {
      const file = Bun.file(`./dist${path}`);
      const exists = await file.exists();
      if (exists) return new Response(file);
    }

    // ── API Routes ──────────────────────────────────────────

    // Health
    if (path === "/api/health" && req.method === "GET") {
      return proxyGet("/health");
    }

    // Get all regions
    if (path === "/api/regions" && req.method === "GET") {
      return proxyGet("/api/regions");
    }

    // Get region summaries
    const summariesMatch = path.match(/^\/api\/regions\/([^/]+)\/summaries$/);
    if (summariesMatch && req.method === "GET") {
      return proxyGet(`/api/regions/${summariesMatch[1]}/summaries`);
    }

    // Generate summary for a region
    const generateMatch = path.match(/^\/api\/regions\/([^/]+)\/generate$/);
    if (generateMatch && req.method === "POST") {
      const res = await fetch(`${ORCH_BASE}/api/regions/${generateMatch[1]}/generate`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: req.body,
      });
      const data = await res.json();
      return json(data, res.status);
    }

    // Invalidate summary for a region
    const invalidateMatch = path.match(/^\/api\/regions\/([^/]+)\/invalidate$/);
    if (invalidateMatch && req.method === "POST") {
      const res = await fetch(`${ORCH_BASE}/api/regions/${invalidateMatch[1]}/invalidate`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: req.body,
      });
      
      // Handle both JSON and non-JSON responses
      const contentType = res.headers.get("content-type");
      if (contentType && contentType.includes("application/json")) {
        const data = await res.json();
        return json(data, res.status);
      } else {
        const text = await res.text();
        return new Response(JSON.stringify({ message: text || "Summary invalidated" }), {
          status: res.status,
          headers: { "Content-Type": "application/json", ...CORS },
        });
      }
    }

    // Region pipeline status
    const regionStatusMatch = path.match(/^\/api\/regions\/([^/]+)\/status$/);
    if (regionStatusMatch && req.method === "GET") {
      return proxyGet(`/api/regions/${regionStatusMatch[1]}/status`);
    }

    // Batch status
    const batchMatch = path.match(/^\/api\/batches\/([^/]+)\/status$/);
    if (batchMatch && req.method === "GET") {
      return proxyGet(`/api/batches/${batchMatch[1]}/status`);
    }

    // Pipeline stats
    if (path === "/api/pipeline/stats" && req.method === "GET") {
      return proxyGet("/api/pipeline/stats");
    }

    // Config
    if (path === "/api/config") {
      if (req.method === "GET") return proxyGet("/api/config");
      if (req.method === "PATCH") {
        const body = await req.json();
        const res = await fetch(`${ORCH_BASE}/api/config`, {
          method: "PATCH",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        });
        const data = await res.json();
        return json(data, res.status);
      }
    }

    // Chunk source
    const chunkMatch = path.match(/^\/api\/chunks\/([^/]+)\/source$/);
    if (chunkMatch && req.method === "GET") {
      return proxyGet(`/api/chunks/${chunkMatch[1]}/source`);
    }

    return json({ error: "Not found" }, 404);
  },
});

console.log("🧠 Brain Region server running on http://localhost:3001");
