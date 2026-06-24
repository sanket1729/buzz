import { HatGlasses } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

/**
 * Composer toolbar button that arms anonymous send and opens the ring picker.
 * The mask mark is the universal "anonymous" signal (UX §2.2, §3.1); an active
 * dot + brand color make the armed state unmistakable so a user never sends
 * anonymously by accident.
 */
export function AnonComposeToggle({
  armed,
  disabled,
  onClick,
}: {
  armed: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label="Send anonymously"
          aria-pressed={armed}
          className={cn("relative rounded-full", armed && "text-anon-verified")}
          disabled={disabled}
          onClick={onClick}
          size="icon"
          type="button"
          variant="ghost"
        >
          <HatGlasses aria-hidden />
          {armed ? (
            <span className="absolute bottom-1 right-1 h-1.5 w-1.5 rounded-full bg-anon-verified" />
          ) : null}
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        <p className="font-medium">Send anonymously</p>
        <p className="text-muted-foreground">
          Signed by you, shown as one of a ring of members.
        </p>
      </TooltipContent>
    </Tooltip>
  );
}
