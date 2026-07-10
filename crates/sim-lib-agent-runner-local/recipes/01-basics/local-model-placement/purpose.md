# Local model placement (descriptor)

Documents how a local model runner advertises its placement (an `Export::Site`) so `realize` can
target it. Placement and model execution happen through the eval fabric outside the cookbook
sandbox eval stack, so the surface is documented rather than run.
