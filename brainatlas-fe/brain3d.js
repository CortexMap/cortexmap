// BrainBrowser 3D Surface Viewer
console.log('🚀 brain3d.js script loaded');
console.log('📦 jQuery available:', typeof $ !== 'undefined');
console.log('🧠 BrainBrowser available:', typeof BrainBrowser !== 'undefined');

$(function() {
  "use strict";

  console.log('✅ DOM ready, initializing BrainBrowser...');

  var loading_div = $("#loading");
  console.log('🔍 Loading div found:', loading_div.length);
  
  function showLoading() { 
    console.log('👁️ Showing loading...');
    loading_div.show(); 
  }
  function hideLoading() { 
    console.log('👁️ Hiding loading...');
    loading_div.hide(); 
  }

  // Check WebGL support
  console.log('🔍 Checking WebGL support...');
  
  // Check if BrainBrowser loaded
  if (typeof BrainBrowser === 'undefined') {
    console.error('❌ BrainBrowser library failed to load!');
    console.error('This is likely due to CORS/OpaqueResponseBlocking by the browser');
    $("#brainbrowser").html(
      '<div style="color: #e8ecf2; text-align: center; padding: 40px; font-family: \'Space Mono\', monospace; max-width: 600px; margin: 0 auto;">' +
      '<div style="font-size: 48px; margin-bottom: 20px;">🧠</div>' +
      '<h3 style="color: #ff6b6b; margin-bottom: 16px;">BrainBrowser Library Blocked</h3>' +
      '<p style="color: #888; font-size: 14px; line-height: 1.6; margin-bottom: 20px;">' +
      'Your browser blocked the BrainBrowser library due to security policies (OpaqueResponseBlocking). ' +
      'This is a known issue with some browsers and the BrainBrowser CDN.' +
      '</p>' +
      '<div style="background: rgba(79, 195, 247, 0.1); border: 1px solid rgba(79, 195, 247, 0.3); border-radius: 8px; padding: 20px; margin: 24px 0; text-align: left;">' +
      '<div style="color: #4fc3f7; font-weight: 700; margin-bottom: 12px;">💡 Solutions:</div>' +
      '<div style="color: #c8d0dc; font-size: 13px; line-height: 1.8;">' +
      '<strong style="color: #e8ecf2;">Option 1:</strong> Try a different browser (Chrome/Firefox recommended)<br>' +
      '<strong style="color: #e8ecf2;">Option 2:</strong> Visit the official BrainBrowser demo:<br>' +
      '<a href="https://brainbrowser.cbrain.mcgill.ca/surface-viewer" target="_blank" style="color: #4fc3f7; margin-left: 20px;">https://brainbrowser.cbrain.mcgill.ca/surface-viewer</a><br>' +
      '<strong style="color: #e8ecf2;">Option 3:</strong> Disable CORS restrictions (for development only)' +
      '</div>' +
      '</div>' +
      '<p style="color: #4fc3f7; font-size: 14px; margin-top: 24px;">' +
      '<a href="index.html" style="color: inherit; text-decoration: none; padding: 10px 20px; background: rgba(79, 195, 247, 0.1); border: 1px solid #4fc3f7; border-radius: 4px; display: inline-block;">← Return to Sunburst View</a>' +
      '</p></div>'
    );
    return;
  }
  
  if (!BrainBrowser.WEBGL_ENABLED) {
    console.error('❌ WebGL is not supported!');
    $("#brainbrowser").html(
      '<div style="color: #ff6b6b; text-align: center; padding: 40px; font-family: \'Space Mono\', monospace;">' +
      '<h3>WebGL Not Supported</h3>' +
      '<p style="color: #888; font-size: 14px; margin-top: 12px;">' +
      'Your browser does not support WebGL, which is required for 3D visualization.' +
      '</p></div>'
    );
    return;
  }
  console.log('✅ WebGL is supported');

  // Verify container exists
  var container = $("#brainbrowser");
  console.log('🔍 BrainBrowser container found:', container.length);
  console.log('🔍 Container dimensions:', container.width(), 'x', container.height());
  
  if (container.length === 0) {
    console.error('❌ BrainBrowser container not found!');
    return;
  }

  // Start the Surface Viewer
  console.log('🎬 Starting BrainBrowser.SurfaceViewer...');
  try {
    window.viewer = BrainBrowser.SurfaceViewer.start("brainbrowser", function(viewer) {
    console.log('✅ BrainBrowser viewer callback triggered');
    console.log('🔍 Viewer object:', viewer);

    // Event listeners
    BrainBrowser.events.addEventListener("error", function(error) {
      console.error('❌ BrainBrowser error:', error);
      hideLoading();
    });

    viewer.addEventListener("displaymodel", function(event) {
      console.log('📦 Model displayed:', event.model);
      hideLoading();
      updatePanelInfo({
        name: 'Human Brain Surface',
        vertices: event.model.children[0] ? event.model.children[0].geometry.attributes.position.count : 'Loading...'
      });
    });

    viewer.addEventListener("loadcolormap", function(event) {
      console.log('🎨 Color map loaded:', event.color_map);
    });

    viewer.addEventListener("loadintensitydata", function(event) {
      console.log('📊 Intensity data loaded:', event.intensity_data);
      hideLoading();
    });

    viewer.addEventListener("clearscreen", function() {
      console.log('🧹 Screen cleared');
    });

    // Start rendering
    viewer.render();
    console.log('🎬 Rendering started');

    // Load the default color map
    viewer.loadColorMapFromURL("https://brainbrowser.cbrain.mcgill.ca/color-maps/spectral.txt", {
      complete: function() {
        console.log('✅ Color map loaded');
      }
    });

    // Load the brain model
    showLoading();
    console.log('📥 Loading brain model from URL...');
    console.log('🔗 Model URL: https://brainbrowser.cbrain.mcgill.ca/models/brain-surface.obj');
    
    viewer.loadModelFromURL("https://brainbrowser.cbrain.mcgill.ca/models/brain-surface.obj", {
      format: "mniobj",
      parse: { split: true },
      complete: function() {
        console.log('✅ Brain model load complete callback');
        console.log('🔍 Viewer models:', viewer.model_data);
        
        // Load intensity data
        console.log('📥 Loading intensity data...');
        viewer.loadIntensityDataFromURL("https://brainbrowser.cbrain.mcgill.ca/models/dti.txt", {
          name: "DTI Data",
          complete: function() {
            console.log('✅ Intensity data load complete');
            hideLoading();
          },
          error: function(error) {
            console.error('❌ Failed to load intensity data:', error);
            hideLoading();
          }
        });
      },
      error: function(error) {
        console.error('❌ Failed to load brain model:', error);
        hideLoading();
        $("#brainbrowser").html(
          '<div style="color: #ff6b6b; text-align: center; padding: 40px; font-family: \'Space Mono\', monospace;">' +
          '<h3>Failed to Load Brain Model</h3>' +
          '<p style="color: #888; font-size: 14px; margin-top: 12px;">' +
          'Error: ' + (error.message || error) +
          '</p></div>'
        );
      }
    });

    // UI Controls
    $("#wireframe").change(function() {
      viewer.setWireframe($(this).is(":checked"));
      console.log('🔲 Wireframe:', $(this).is(":checked"));
    });

    $("#autorotate").change(function() {
      var enabled = $(this).is(":checked");
      viewer.autorotate.x = false;
      viewer.autorotate.y = enabled;
      viewer.autorotate.z = false;
      console.log('🔄 Auto-rotate:', enabled);
    });

    // Pick handler (Shift + Click)
    $("#brainbrowser").click(function(event) {
      if (!event.shiftKey) return;
      
      var pick_info = viewer.pick(viewer.mouse.x, viewer.mouse.y);
      
      if (pick_info) {
        console.log('🎯 Picked vertex:', pick_info);
        showVertexModal(pick_info, viewer);
      }
    });

    // Window resize
    $(window).resize(function() {
      viewer.updateViewport();
    });
  });
  } catch (error) {
    console.error('❌ Failed to start BrainBrowser:', error);
    console.error('Stack trace:', error.stack);
    $("#brainbrowser").html(
      '<div style="color: #ff6b6b; text-align: center; padding: 40px; font-family: \'Space Mono\', monospace;">' +
      '<h3>Failed to Initialize 3D Viewer</h3>' +
      '<p style="color: #888; font-size: 14px; margin-top: 12px;">' +
      'Error: ' + error.message +
      '</p>' +
      '<p style="color: #666; font-size: 11px; margin-top: 8px; font-family: monospace;">' +
      error.stack +
      '</p></div>'
    );
  }

  // Update side panel information
  function updatePanelInfo(info) {
    $("#panel-name").text(info.name || '3D Brain Surface');
    
    if (info.vertices) {
      var html = '<div class="info-row">' +
        '<span class="info-label">Vertices:</span> ' +
        '<span>' + info.vertices.toLocaleString() + '</span>' +
        '</div>';
      $("#panel-body").prepend(html);
    }
  }

  // Show vertex information in modal
  function showVertexModal(pick_info, viewer) {
    var model_data = viewer.model_data.get(pick_info.object.userData.model_name);
    var intensity_data = model_data ? model_data.intensity_data[0] : null;
    var value = intensity_data ? intensity_data.values[pick_info.index] : 0;
    
    var color = viewer.color_map ? 
      viewer.color_map.colorFromValue(value, {
        min: intensity_data ? intensity_data.range_min : 0,
        max: intensity_data ? intensity_data.range_max : 1
      }) : 
      { r: 79, g: 195, b: 247 };

    $("#modal-title").text("Vertex " + pick_info.index);
    $("#modal-color").css("background", "rgb(" + color.r + "," + color.g + "," + color.b + ")");
    $("#modal-acro").text("V" + pick_info.index);
    $("#modal-id").text("Intensity: " + value.toFixed(4));
    
    $("#modal-body").html(
      '<div style="padding: 20px; color: var(--text); line-height: 1.7;">' +
      '<div class="info-row">' +
      '<span class="info-label">Vertex Index:</span> ' +
      '<span>' + pick_info.index + '</span>' +
      '</div>' +
      '<div class="info-row">' +
      '<span class="info-label">Position X:</span> ' +
      '<span>' + pick_info.point.x.toFixed(4) + '</span>' +
      '</div>' +
      '<div class="info-row">' +
      '<span class="info-label">Position Y:</span> ' +
      '<span>' + pick_info.point.y.toFixed(4) + '</span>' +
      '</div>' +
      '<div class="info-row">' +
      '<span class="info-label">Position Z:</span> ' +
      '<span>' + pick_info.point.z.toFixed(4) + '</span>' +
      '</div>' +
      '<div class="info-row">' +
      '<span class="info-label">Intensity Value:</span> ' +
      '<span>' + value.toFixed(4) + '</span>' +
      '</div>' +
      '<div style="margin-top: 16px; padding-top: 16px; border-top: 1px solid var(--border); color: var(--text-dim); font-size: 11px;">' +
      '<strong style="color: var(--text);">About this data:</strong><br>' +
      'This is demo DTI (Diffusion Tensor Imaging) data mapped onto the brain surface, ' +
      'showing intensity values at each vertex of the 3D model.' +
      '</div>' +
      '</div>'
    );
    
    $("#modal-overlay").addClass("visible");
  }

  // Modal close handlers
  $("#modal-close").click(function() {
    $("#modal-overlay").removeClass("visible");
  });

  $("#modal-overlay").click(function(e) {
    if (e.target.id === "modal-overlay") {
      $("#modal-overlay").removeClass("visible");
    }
  });
});
