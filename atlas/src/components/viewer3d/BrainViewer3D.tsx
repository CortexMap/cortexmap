import { useEffect, useRef, useState } from 'react';
import { Canvas, useThree, useFrame } from '@react-three/fiber';
import { TrackballControls } from '@react-three/drei';
import * as THREE from 'three';
import { useAtlasStore } from '../../store/atlasStore';
import { useSelectRegion } from '../../hooks/useSelectRegion';
import { findNode, getAncestorPath } from '../../utils/treeUtils';
import { BRAIN_CENTER, BRAIN_SCALE } from '../../utils/objLoader';
import styles from './BrainViewer3D.module.css';

const DEFAULT_CAMERA_POS = new THREE.Vector3(0, 0, 12);
const DEFAULT_CAMERA_TARGET = new THREE.Vector3(0, 0, 0);

// Shared flag: when true, CameraController is running a fly-to animation.
// KeyboardRotator and mouse input cancel this immediately.
let cameraAnimating = false;

export function BrainViewer3D() {
  const { loadInitialMeshes, meshesLoading, meshLoadProgress } = useAtlasStore();
  const selectRegion = useSelectRegion();
  const [toast, setToast] = useState<{ regionName: string; parentName: string } | null>(null);
  const toastTimeout = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    loadInitialMeshes();
  }, [loadInitialMeshes]);

  // Fallback: when a region is selected but has no mesh, find nearest ancestor with mesh
  const { selectedStructureId, loadedMeshes, ontology, fallback3d, clearFallback3d } = useAtlasStore();

  useEffect(() => {
    if (fallback3d) {
      setToast({ regionName: fallback3d.regionName, parentName: fallback3d.parentName });
      if (toastTimeout.current) clearTimeout(toastTimeout.current);
      toastTimeout.current = setTimeout(() => {
        setToast(null);
        clearFallback3d();
      }, 4000);
    }
    return () => {
      if (toastTimeout.current) clearTimeout(toastTimeout.current);
    };
  }, [fallback3d, clearFallback3d]);

  // Detect when selected structure has no mesh and find fallback
  useEffect(() => {
    if (selectedStructureId === null || !ontology) return;
    // If mesh exists for the selected structure, no fallback needed
    if (loadedMeshes.has(selectedStructureId)) return;

    // Walk up ancestors to find nearest one with a mesh
    const ancestors = getAncestorPath(ontology, selectedStructureId);
    // ancestors is root->parent path (not including the node itself)
    // Reverse to check closest parent first
    const reversedAncestors = [...ancestors].reverse();

    let fallbackId: number | null = null;
    for (const ancestorId of reversedAncestors) {
      if (loadedMeshes.has(ancestorId)) {
        fallbackId = ancestorId;
        break;
      }
    }

    if (fallbackId !== null) {
      const selectedNode = findNode(ontology, selectedStructureId);
      const parentNode = findNode(ontology, fallbackId);
      const regionName = selectedNode ? `${selectedNode.a} (${selectedNode.n})` : `ID ${selectedStructureId}`;
      const parentName = parentNode ? `${parentNode.a} (${parentNode.n})` : `ID ${fallbackId}`;

      // Make the fallback mesh visible if not already
      const visible = new Set(useAtlasStore.getState().visibleMeshIds);
      visible.add(fallbackId);
      useAtlasStore.setState({ visibleMeshIds: visible, fallback3d: { regionName, parentName, parentId: fallbackId } });
    }
  }, [selectedStructureId, loadedMeshes, ontology]);

  // Track which arrow keys are currently held
  const keysPressed = useRef(new Set<string>());

  // Keyboard shortcuts (non-arrow)
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;

      const store = useAtlasStore.getState();
      switch (e.key.toLowerCase()) {
        case 'w':
          store.setViewer3dMode(store.viewer3dMode === 'wireframe' ? 'regions' : 'wireframe');
          break;
        case ' ':
          e.preventDefault();
          store.setAutoRotate(!store.autoRotate);
          break;
        case 'h':
          store.setHighlight3d(!store.highlight3d);
          break;
      }

      // Track arrow keys
      if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) {
        e.preventDefault();
        keysPressed.current.add(e.key);
        // Store ctrl state for zoom
        if (e.ctrlKey || e.metaKey) keysPressed.current.add('ctrl');
      }
    };
    const upHandler = (e: KeyboardEvent) => {
      keysPressed.current.delete(e.key);
      if (!e.ctrlKey && !e.metaKey) keysPressed.current.delete('ctrl');
    };
    window.addEventListener('keydown', handler);
    window.addEventListener('keyup', upHandler);
    return () => {
      window.removeEventListener('keydown', handler);
      window.removeEventListener('keyup', upHandler);
    };
  }, []);

  return (
    <div className={styles.container} tabIndex={0}>
      {meshesLoading && (
        <div className={styles.loadingOverlay}>
          <div className={styles.loadingBar}>
            <div
              className={styles.loadingFill}
              style={{ width: `${meshLoadProgress.total ? (meshLoadProgress.loaded / meshLoadProgress.total) * 100 : 0}%` }}
            />
          </div>
          <span className={styles.loadingText}>
            Loading meshes ({meshLoadProgress.loaded}/{meshLoadProgress.total})
          </span>
        </div>
      )}
      <Canvas
        camera={{ position: [0, 0, 12], fov: 50, near: 0.1, far: 100 }}
        gl={{ antialias: true, alpha: false }}
        style={{ background: '#1a1a2e' }}
        onPointerMissed={() => {
          // Click on empty space = deselect
          selectRegion(null);
        }}
      >
        <SceneLights />
        <SceneContent />
        <CameraController />
        <KeyboardRotator keysPressed={keysPressed} />
        <AutoRotator />
        <OrbitControlsWrapper />
      </Canvas>
      {toast && (
        <div className={styles.toast}>
          <div className={styles.toastIcon}>&#9432;</div>
          <div className={styles.toastText}>
            <strong>{toast.regionName}</strong> has no 3D mesh.
            Showing parent: <strong>{toast.parentName}</strong>
          </div>
        </div>
      )}
    </div>
  );
}

function SceneLights() {
  return (
    <>
      <ambientLight intensity={0.4} />
      <directionalLight position={[10, 10, 10]} intensity={0.8} />
      <directionalLight position={[-10, -5, -10]} intensity={0.3} />
    </>
  );
}

function SceneContent() {
  const {
    loadedMeshes, visibleMeshIds, selectedStructureId, hoveredStructureId,
    meshOpacity, viewer3dMode, ontology, hoverStructure,
    focused3dRegionId, highlight3d, checked3dIds, fallback3d,
  } = useAtlasStore();
  const selectRegion = useSelectRegion();

  const shellGeometry = loadedMeshes.get(997);
  const hasChecked = checked3dIds.size > 0;

  // Determine which mesh IDs to render
  const renderIds: number[] = [];
  visibleMeshIds.forEach((id) => {
    if (id !== 997) renderIds.push(id);
  });

  return (
    <group
      scale={[BRAIN_SCALE, BRAIN_SCALE, BRAIN_SCALE]}
      position={[
        -BRAIN_CENTER.x * BRAIN_SCALE,
        -BRAIN_CENTER.y * BRAIN_SCALE,
        -BRAIN_CENTER.z * BRAIN_SCALE,
      ]}
    >
      {/* Brain shell -- dimmer when focused or when checkboxes are active */}
      {shellGeometry && (
        <mesh geometry={shellGeometry}>
          <meshPhysicalMaterial
            transparent
            opacity={(focused3dRegionId || hasChecked) ? 0.04 : 0.08}
            color="#a0a0c0"
            wireframe={viewer3dMode === 'wireframe'}
            side={THREE.DoubleSide}
            depthWrite={false}
          />
        </mesh>
      )}

      {/* Region meshes */}
      {renderIds.map((id) => {
        const geo = loadedMeshes.get(id);
        if (!geo) return null;

        const isSelected = id === selectedStructureId || (fallback3d?.parentId === id && !loadedMeshes.has(selectedStructureId ?? -1));
        const isHovered = highlight3d && id === hoveredStructureId;

        // Get color from ontology
        let color = '#8888aa';
        if (ontology) {
          const node = findNode(ontology, id);
          if (node?.c) color = `#${node.c}`;
        }

        const finalColor = isSelected ? '#7c3aed' : isHovered ? '#a78bfa' : color;
        const finalOpacity = isSelected ? 0.9 : isHovered ? 0.85 : meshOpacity;

        return (
          <mesh
            key={id}
            geometry={geo}
            onClick={(e) => { e.stopPropagation(); selectRegion(id); }}
            onPointerOver={(e) => { e.stopPropagation(); if (highlight3d) hoverStructure(id); }}
            onPointerOut={() => { if (highlight3d) hoverStructure(null); }}
          >
            <meshPhysicalMaterial
              transparent
              opacity={finalOpacity}
              color={finalColor}
              wireframe={viewer3dMode === 'wireframe'}
              emissive={isSelected ? '#7c3aed' : isHovered ? '#a78bfa' : '#000000'}
              emissiveIntensity={isSelected ? 0.3 : isHovered ? 0.15 : 0}
              roughness={0.4}
              metalness={0.1}
              side={THREE.DoubleSide}
            />
          </mesh>
        );
      })}
    </group>
  );
}

function CameraController() {
  const { selectedStructureId, loadedMeshes } = useAtlasStore();
  const { camera } = useThree();
  const targetRef = useRef(new THREE.Vector3());

  useEffect(() => {
    if (selectedStructureId === null) return;

    const geo = loadedMeshes.get(selectedStructureId);
    if (!geo || !geo.boundingSphere) return;

    // Calculate world position of the mesh center
    const center = geo.boundingSphere.center.clone();
    center.multiplyScalar(BRAIN_SCALE);
    center.x -= BRAIN_CENTER.x * BRAIN_SCALE;
    center.y -= BRAIN_CENTER.y * BRAIN_SCALE;
    center.z -= BRAIN_CENTER.z * BRAIN_SCALE;

    targetRef.current.copy(center);
    cameraAnimating = true;
  }, [selectedStructureId, loadedMeshes, camera]);

  // Listen for 'R' key to reset camera
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.key.toLowerCase() === 'r') {
        cameraAnimating = false;
        targetRef.current.copy(DEFAULT_CAMERA_TARGET);
        // Reset camera to default position and orientation
        camera.position.copy(DEFAULT_CAMERA_POS);
        camera.up.set(0, 1, 0);
        camera.lookAt(DEFAULT_CAMERA_TARGET);
        if (orbitControlsRef) {
          orbitControlsRef.target.copy(DEFAULT_CAMERA_TARGET);
          orbitControlsRef.update();
        }
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [camera]);

  useFrame(() => {
    if (!cameraAnimating) return;

    const dir = new THREE.Vector3().subVectors(targetRef.current, camera.position).normalize();
    const targetPos = targetRef.current.clone().sub(dir.multiplyScalar(8));

    camera.position.lerp(targetPos, 0.03);
    camera.lookAt(targetRef.current);

    if (camera.position.distanceTo(targetPos) < 0.05) {
      cameraAnimating = false;
    }
  });

  return null;
}

function KeyboardRotator({ keysPressed }: { keysPressed: React.RefObject<Set<string>> }) {
  const { camera } = useThree();

  useFrame(() => {
    const keys = keysPressed.current;
    if (!keys || keys.size === 0) return;

    // Cancel any in-progress fly-to/reset animation on user input
    cameraAnimating = false;

    const ROTATE_SPEED = 0.02;
    const ZOOM_SPEED = 0.15;
    const hasCtrl = keys.has('ctrl');

    const target = orbitControlsRef ? orbitControlsRef.target.clone() : new THREE.Vector3(0, 0, 0);
    const offset = camera.position.clone().sub(target);
    const radius = offset.length();

    // Extract camera's actual local axes from its quaternion (no lookAt needed)
    const camRight = new THREE.Vector3(1, 0, 0).applyQuaternion(camera.quaternion);
    const camUp = new THREE.Vector3(0, 1, 0).applyQuaternion(camera.quaternion);

    // Accumulate a combined rotation quaternion
    const combinedQ = new THREE.Quaternion();

    // Left/Right: rotate around camera's local up axis
    if (keys.has('ArrowLeft')) {
      combinedQ.multiply(new THREE.Quaternion().setFromAxisAngle(camUp, ROTATE_SPEED));
    }
    if (keys.has('ArrowRight')) {
      combinedQ.multiply(new THREE.Quaternion().setFromAxisAngle(camUp, -ROTATE_SPEED));
    }

    // Up/Down: rotate around camera's local right axis (or zoom with Ctrl)
    if (keys.has('ArrowUp')) {
      if (hasCtrl) {
        const len = Math.max(3, radius - ZOOM_SPEED);
        offset.normalize().multiplyScalar(len);
      } else {
        combinedQ.multiply(new THREE.Quaternion().setFromAxisAngle(camRight, ROTATE_SPEED));
      }
    }
    if (keys.has('ArrowDown')) {
      if (hasCtrl) {
        const len = Math.min(30, radius + ZOOM_SPEED);
        offset.normalize().multiplyScalar(len);
      } else {
        combinedQ.multiply(new THREE.Quaternion().setFromAxisAngle(camRight, -ROTATE_SPEED));
      }
    }

    // Apply rotation to position offset
    offset.applyQuaternion(combinedQ);
    camera.position.copy(target).add(offset);

    // Apply the INVERSE rotation to the camera orientation so it keeps looking at target
    // (position rotates one way, camera orientation counter-rotates to stay aimed at target)
    camera.quaternion.premultiply(combinedQ.invert());

    // Update camera.up from the rotated quaternion to keep TrackballControls in sync
    camera.up.set(0, 1, 0).applyQuaternion(camera.quaternion);

    // Sync TrackballControls without letting it recalculate lookAt
    if (orbitControlsRef) {
      orbitControlsRef.target.copy(target);
      orbitControlsRef.update();
    }
  });

  return null;
}

// Auto-rotate: TrackballControls doesn't have built-in auto-rotate,
// so we implement it manually using useFrame
function AutoRotator() {
  const { camera } = useThree();
  const { autoRotate } = useAtlasStore();

  useFrame(() => {
    if (!autoRotate || !orbitControlsRef) return;

    const target = orbitControlsRef.target.clone();
    const offset = camera.position.clone().sub(target);

    // Rotate around the camera's local up axis (screen-relative, no flip)
    const camUp = new THREE.Vector3(0, 1, 0).applyQuaternion(camera.quaternion);
    const angle = 0.003;
    const q = new THREE.Quaternion().setFromAxisAngle(camUp, angle);

    offset.applyQuaternion(q);
    camera.position.copy(target).add(offset);

    // Counter-rotate the camera orientation
    camera.quaternion.premultiply(q.invert());
    camera.up.set(0, 1, 0).applyQuaternion(camera.quaternion);

    orbitControlsRef.target.copy(target);
    orbitControlsRef.update();
  });

  return null;
}

// Store a ref to the controls so ControlBar's reset button can use it
let orbitControlsRef: any = null;

function OrbitControlsWrapper() {
  const { autoRotate } = useAtlasStore();
  const controlsRef = useRef<any>(null);

  useEffect(() => {
    orbitControlsRef = controlsRef.current;

    // Cancel fly-to animation when user starts dragging/scrolling
    const controls = controlsRef.current;
    if (controls) {
      const onStart = () => { cameraAnimating = false; };
      controls.addEventListener('start', onStart);
      return () => {
        controls.removeEventListener('start', onStart);
        orbitControlsRef = null;
      };
    }
    return () => { orbitControlsRef = null; };
  });

  // TrackballControls: no gimbal lock, full 360-degree rotation without flipping
  // staticMoving=false gives smooth damping; rotateSpeed controls mouse drag sensitivity
  return (
    <TrackballControls
      ref={controlsRef}
      rotateSpeed={2.0}
      zoomSpeed={1.2}
      panSpeed={0.8}
      staticMoving={false}
      dynamicDampingFactor={0.15}
      minDistance={3}
      maxDistance={30}
      noRotate={false}
      noPan={false}
      noZoom={false}
    />
  );
}

export function resetCameraView() {
  if (orbitControlsRef) {
    orbitControlsRef.object.position.copy(DEFAULT_CAMERA_POS);
    orbitControlsRef.object.up.set(0, 1, 0);
    orbitControlsRef.target.copy(DEFAULT_CAMERA_TARGET);
    orbitControlsRef.object.lookAt(DEFAULT_CAMERA_TARGET);
    orbitControlsRef.update();
  }
}

export function ControlBar() {
  const {
    meshOpacity, setMeshOpacity, viewer3dMode, setViewer3dMode,
    autoRotate, setAutoRotate, selectedStructureId,
    focused3dRegionId, focusOn3dRegion, clearFocus3d,
    ontology, highlight3d, setHighlight3d,
    checked3dIds, clearAllChecked3d,
  } = useAtlasStore();

  // Get the name of the focused region for display
  let focusedName = '';
  if (focused3dRegionId && ontology) {
    const node = findNode(ontology, focused3dRegionId);
    if (node) focusedName = node.a;
  }

  const hasChecked = checked3dIds.size > 0;

  return (
    <div className={styles.controlBar}>
      {/* Focus controls */}
      <div className={styles.controlGroup}>
        {focused3dRegionId ? (
          <>
            <span className={styles.focusLabel}>Focused: {focusedName}</span>
            <button
              className={styles.controlBtn}
              onClick={() => clearFocus3d()}
              title="Show all major regions"
            >
              Show All
            </button>
          </>
        ) : hasChecked ? (
          <button
            className={styles.controlBtn}
            onClick={() => clearAllChecked3d()}
            title="Clear checkbox selections, show all"
          >
            Show All
          </button>
        ) : (
          <button
            className={`${styles.controlBtn} ${selectedStructureId ? '' : styles.disabled}`}
            onClick={() => selectedStructureId && focusOn3dRegion(selectedStructureId)}
            title={selectedStructureId ? 'Focus on selected region and its children' : 'Select a region first'}
            disabled={!selectedStructureId}
          >
            Focus
          </button>
        )}
      </div>

      <div className={styles.divider} />

      <div className={styles.controlGroup}>
        <label className={styles.controlLabel}>Opacity</label>
        <input
          type="range"
          min="0.1"
          max="1"
          step="0.05"
          value={meshOpacity}
          onChange={(e) => setMeshOpacity(Number(e.target.value))}
          className={styles.slider}
        />
      </div>
      <div className={styles.controlGroup}>
        <button
          className={`${styles.controlBtn} ${viewer3dMode === 'regions' ? styles.active : ''}`}
          onClick={() => setViewer3dMode('regions')}
          title="Solid mode"
        >
          Solid
        </button>
        <button
          className={`${styles.controlBtn} ${viewer3dMode === 'wireframe' ? styles.active : ''}`}
          onClick={() => setViewer3dMode('wireframe')}
          title="Wireframe mode (W)"
        >
          Wire
        </button>
      </div>
      <div className={styles.controlGroup}>
        <button
          className={`${styles.controlBtn} ${highlight3d ? styles.active : ''}`}
          onClick={() => setHighlight3d(!highlight3d)}
          title="Toggle hover highlighting (H)"
        >
          Highlight
        </button>
        <button
          className={`${styles.controlBtn} ${autoRotate ? styles.active : ''}`}
          onClick={() => setAutoRotate(!autoRotate)}
          title="Toggle auto-rotate (Space)"
        >
          Rotate
        </button>
        <button
          className={styles.controlBtn}
          onClick={() => resetCameraView()}
          title="Reset camera (R)"
        >
          Reset
        </button>
      </div>
      <div className={styles.shortcuts}>
        <span>H</span> highlight &nbsp; <span>W</span> wire &nbsp; <span>Space</span> rotate &nbsp; <span>R</span> reset &nbsp; <span>Arrows</span> rotate &nbsp; <span>Ctrl+&uarr;&darr;</span> zoom
      </div>
    </div>
  );
}
