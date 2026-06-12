import React from "react";
import Image from "next/image";

interface LogoProps {
    isCollapsed: boolean;
}

// Static brand mark. (The "About" dialog was removed.)
const Logo = React.forwardRef<HTMLButtonElement, LogoProps>(({ isCollapsed }, _ref) => {
  return isCollapsed ? (
    <div className="flex items-center justify-start mb-2">
      <Image src="/logo-collapsed.png" alt="Oliv Recorder" width={40} height={32} />
    </div>
  ) : (
    <span className="text-lg text-center border rounded-full bg-blue-50 border-white font-semibold text-gray-700 mb-2 block items-center">
      <span>Oliv Recorder</span>
    </span>
  );
});

Logo.displayName = "Logo";

export default Logo;
